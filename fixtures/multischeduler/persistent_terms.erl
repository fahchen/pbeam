-module(persistent_terms).
-export([writer/0, reader/0, eraser/0, erase_reader/0, turns/0,
         trapped_get/0, pair_reference/0, one_reference/0]).

-define(KEYS, 5000).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
pair_reference() ->
    [pair(fun writer/0, fun reader/0), pair(fun eraser/0, fun erase_reader/0)].

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
    [persistent_term:erase({trapped, I}) || I <- lists:seq(1, ?KEYS)],
    _ = persistent_term:erase(late_key),
    case {First, Rest, Killed} of
        {{got, false}, [erase_done, put_done, {got_again, _}], ok} -> ok;
        Other -> Other
    end.

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
