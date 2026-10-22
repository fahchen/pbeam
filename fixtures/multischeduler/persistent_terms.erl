-module(persistent_terms).
-export([snapshot_writer/0, snapshot_reader/0, writer/0, reader/0, eraser/0, erase_reader/0, turns/0,
         trapped_get/0, pair_reference/0, one_reference/0]).

-define(KEYS, 5000).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
pair_reference() ->
    [pair(fun writer/0, fun reader/0), pair(fun eraser/0, fun erase_reader/0),
     pair(fun snapshot_writer/0, fun snapshot_reader/0)].

%% Run the probes of one scheduler on the pinned ERTS.
one_reference() ->
    {turns(), trapped_get()}.

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

-define(SNAPSHOTS, 200).

%% The root that puts a large table, then the keys {snap, 1} to
%% {snap, SNAPSHOTS} in order, while the root of the other scheduler reads
%% the whole table with get/0, which traps.
snapshot_writer() ->
    [persistent_term:put({fill, I}, I) || I <- lists:seq(1, ?KEYS)],
    [persistent_term:put({snap, I}, I) || I <- lists:seq(1, ?SNAPSHOTS)],
    ok.

%% The root that reads the whole table until it holds each {snap, I}. Each
%% snapshot holds the keys of a prefix of the puts.
snapshot_reader() ->
    snapshot_reader(1000000).

snapshot_reader(0) ->
    timeout;
snapshot_reader(N) ->
    Snaps = lists:sort([I || {{snap, I}, _} <- persistent_term:get()]),
    case Snaps =:= lists:seq(1, length(Snaps)) of
        false -> {torn, Snaps};
        true when length(Snaps) =:= ?SNAPSHOTS -> ok;
        true -> erlang:yield(), snapshot_reader(N - 1)
    end.

%% The root that puts a term that the root of the other scheduler reads.
writer() ->
    ok = persistent_term:put(shared_key, {written, 7}),
    {written, 7} = persistent_term:get(shared_key),
    ok.

%% The root that waits until the term of the other root is visible.
reader() ->
    wait_for(shared_key, {written, 7}, 1000000).

%% The root that puts a term and erases it after the other root saw it.
eraser() ->
    ok = persistent_term:put(erase_key, here),
    wait_for(erase_seen, true, 1000000),
    true = persistent_term:erase(erase_key),
    false = persistent_term:erase(erase_key),
    ok.

%% The root that sees the term of the other root and then its erase.
erase_reader() ->
    ok = wait_for(erase_key, here, 1000000),
    ok = persistent_term:put(erase_seen, true),
    wait_for(erase_key, gone, 1000000).

wait_for(_Key, _Value, 0) ->
    timeout;
wait_for(Key, Value, N) ->
    case persistent_term:get(Key, gone) of
        Value -> ok;
        _ -> erlang:yield(), wait_for(Key, Value, N - 1)
    end.

%% Count the turns of another process while `persistent_term:put/2`
%% suspends the caller. Return ok when the other process ran at least two
%% turns, as with `erl +S 1`.
turns() ->
    Self = self(),
    {Ticker, _} = spawn_monitor(fun() -> tick(Self) end),
    receive tick -> ok end,
    flush(),
    ok = persistent_term:put(turns_key, 1),
    Ticks = count(0),
    Ticker ! stop,
    receive {'DOWN', _, process, Ticker, _} -> ok end,
    flush(),
    if Ticks >= 2 -> ok; true -> {ticks, Ticks} end.

tick(Parent) ->
    Parent ! tick,
    receive stop -> ok after 0 -> erlang:yield(), tick(Parent) end.

flush() ->
    receive tick -> flush() after 0 -> ok end.

count(N) ->
    receive tick -> count(N + 1) after 0 -> N end.

%% Put a key, erase a key, and read all keys while `persistent_term:get/0`
%% of another process traps. Each call waits until the last chunk of the
%% get, so the get does not see the change, and the get ends first. Then a
%% process that exits while its `get/0` traps gives the permission back.
trapped_get() ->
    [persistent_term:put({trapped, I}, I) || I <- lists:seq(1, ?KEYS)],
    Self = self(),
    _ = spawn(fun() ->
                  All = persistent_term:get(),
                  Self ! {got, lists:keymember(late_key, 1, All)}
              end),
    _ = spawn(fun() ->
                  ok = persistent_term:put(late_key, 1),
                  Self ! put_done
              end),
    _ = spawn(fun() ->
                  true = persistent_term:erase({trapped, 1}),
                  Self ! erase_done
              end),
    _ = spawn(fun() ->
                  All = persistent_term:get(),
                  Self ! {got_again, lists:keymember(late_key, 1, All)}
              end),
    First = result(),
    Rest = lists:sort([result(), result(), result()]),
    Killed = killed_get(),
    High = high_wait(),
    Reader = high_reader(),
    Order = wait_order(),
    [persistent_term:erase({trapped, I}) || I <- lists:seq(1, ?KEYS)],
    _ = persistent_term:erase(late_key),
    case {First, Rest, Killed, High, Reader, Order} of
        {{got, false}, [erase_done, put_done, {got_again, _}], ok, ok, ok, first} -> ok;
        Other -> Other
    end.

%% A process of priority high that waits for the permission suspends, so
%% the get of priority normal ends.
high_wait() ->
    Self = self(),
    _ = spawn(fun() ->
                  All = persistent_term:get(),
                  Self ! {got, lists:keymember(high_key, 1, All)}
              end),
    erlang:yield(),
    _ = spawn_opt(fun() ->
                      ok = persistent_term:put(high_key, 1),
                      Self ! put_done
                  end, [{priority, high}]),
    case {result(), result()} of
        {{got, false}, put_done} -> ok;
        Other -> Other
    end.

%% A process of priority high reads until the put of a process of priority
%% normal is visible, and then puts a key itself. Its put does not wait for
%% a turn of the first process.
high_reader() ->
    Self = self(),
    _ = spawn_opt(fun() ->
                      ok = wait_for(reader_key, 1, 1000000),
                      ok = persistent_term:put(reader_done, 1),
                      Self ! reader_done
                  end, [{priority, high}]),
    ok = persistent_term:put(reader_key, 1),
    receive reader_done -> ok after 60000 -> timeout end.

%% Two puts of one key wait while a get/0 traps. ERTS resumes the last
%% waiter first, so the first put runs last.
wait_order() ->
    Self = self(),
    _ = spawn(fun() -> _ = persistent_term:get(), Self ! got end),
    _ = spawn(fun() -> ok = persistent_term:put(order_key, first), Self ! put end),
    _ = spawn(fun() -> ok = persistent_term:put(order_key, second), Self ! put end),
    [receive M -> M after 60000 -> timeout end || M <- [got, put, put]],
    persistent_term:get(order_key).

killed_get() ->
    Getter = spawn(fun() -> persistent_term:get() end),
    erlang:yield(),
    exit(Getter, kill),
    persistent_term:put(after_kill, 1).

result() ->
    receive
        {got, _} = Got -> Got;
        {got_again, _} = Got -> Got;
        put_done -> put_done;
        erase_done -> erase_done
    after 60000 -> timeout
    end.
