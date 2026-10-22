-module(dirty_nif_probe).
-export([threads/0, signals/1, signals_with/3, kill/1, kill_with/3, queued/1,
         raise/1, unfinished/1, waits/1, priorities/1, priorities_with/4, first_root/0, second_root/0,
         relay_wait/0, relay_send/0, load/0]).

-nifs([cpu_thread/0, io_thread/0, cpu_wait/1, started/1, release/1, cpu_raise/1,
       cpu_sequence/0]).

%% The test declares each NIF as a PBeam native, so its binding runs
%% without `erlang:load_nif/2`.
-pbeam_native([{cpu_thread, 0}, {io_thread, 0}, {cpu_wait, 1}, {started, 1},
               {release, 1}, {cpu_raise, 1}, {cpu_sequence, 0}]).

%% The test registers each NIF of this module as a Rust native function:
%% `cpu_thread/0`, `cpu_wait/1`, `cpu_raise/1`, and `cpu_sequence/0` with
%% the job `CpuBound`,
%% `io_thread/0` with the job `IoBound`, and `started/1` and `release/1`
%% with the job `Normal`. `cpu_wait(Key)` marks `Key` as started, waits until
%% `release(Key)`, and returns `ok`, as `erts_debug:dirty_cpu(wait, Ms)`
%% returns `ok` after `Ms` milliseconds on ERTS. `cpu_sequence/0` returns the
%% next number of a counter, so its value gives the start order of the calls.

%% The names of the threads that run a `CpuBound` and an `IoBound` call.
threads() -> {cpu_thread(), io_thread()}.

signals(Key) ->
    signals_with(fun() -> cpu_wait(Key) end, fun() -> await(Key) end,
                 fun() -> release(Key) end).

%% The state and the signal handling of a caller while its dirty call runs.
%% On ERTS, `Call` is `erts_debug:dirty_cpu(wait, 300)`, `Started` sleeps
%% 50 ms, and `Release` does nothing.
signals_with(Call, Started, Release) ->
    Self = self(),
    Caller = spawn(fun() -> Self ! {done, self(), Call()} end),
    Started(),
    spawn(fun() -> Self ! other_ran end),
    Other = receive other_ran -> ok end,
    Empty = process_info(Caller, message_queue_len),
    Caller ! hello,
    Queued = process_info(Caller, message_queue_len),
    Status = lists:keyfind(status, 1, process_info(Caller)),
    exit(Caller, normal),
    Alive = is_process_alive(Caller),
    Release(),
    Result = receive {done, Caller, Value} -> Value end,
    {Other, Empty, Queued, Status, Alive, Result}.

kill(Key) ->
    kill_with(fun() -> cpu_wait(Key) end, fun() -> await(Key) end,
              fun() -> release(Key) end).

%% A kill of a caller while its dirty call runs. On ERTS, `Call` is
%% `erts_debug:dirty_cpu(wait, 300)`, `Started` sleeps 50 ms, and `Release`
%% does nothing.
kill_with(Call, Started, Release) ->
    Self = self(),
    Caller = spawn(fun() -> Self ! {done, Call()} end),
    Started(),
    {Linked, LinkMonitor} =
        spawn_monitor(fun() -> link(Caller), Self ! linked, receive after infinity -> ok end end),
    receive linked -> ok end,
    Monitor = monitor(process, Caller),
    exit(Caller, kill),
    Down = receive {'DOWN', Monitor, process, Caller, Reason} -> Reason end,
    LinkDown = receive {'DOWN', LinkMonitor, process, Linked, LinkReason} -> LinkReason end,
    Release(),
    Late = receive {done, _} -> late after 500 -> no_late end,
    {Down, LinkDown, Late}.

%% With one dirty CPU thread, a second call waits in the queue. A kill of
%% its caller drops the call: it does not run. The third call runs after
%% the thread skips the dropped call. The `'DOWN'` message comes after the
%% caller handled the kill, so the release comes after the drop.
queued(Key) ->
    Self = self(),
    spawn(fun() -> cpu_wait(Key) end),
    await(Key),
    Second = spawn(fun() -> Self ! calling, cpu_wait(Key + 1) end),
    receive calling -> ok end,
    Status = lists:keyfind(status, 1, process_info(Second)),
    Monitor = monitor(process, Second),
    exit(Second, kill),
    receive {'DOWN', Monitor, process, Second, killed} -> ok end,
    spawn(fun() -> Self ! {third, cpu_wait(Key + 2)} end),
    release(Key),
    await(Key + 2),
    Ran = started(Key + 1),
    release(Key + 2),
    Third = receive {third, Value} -> Value end,
    {Status, Ran, Third}.

priorities(Key) ->
    priorities_with(fun() -> cpu_wait(Key) end, fun() -> await(Key) end,
                    fun() -> cpu_sequence() end, fun() -> release(Key) end).

%% The start order of the dirty calls of callers with each priority, which
%% wait in the queue while `Busy` holds the only dirty CPU thread. Each
%% caller makes its call before the next caller starts. The tags come in the
%% order of the values of `Call`. On ERTS with `+S 1 +SDcpu 1:1`, `Busy` is
%% `erts_debug:dirty_cpu(wait, 300)`, `Started` sleeps 50 ms, `Call` is
%% `erts_debug:dirty_cpu(wait, 20)` and then `erlang:monotonic_time()`, and
%% `Release` does nothing.
priorities_with(Busy, Started, Call, Release) ->
    Self = self(),
    spawn(Busy),
    Started(),
    Callers = [{normal, n1}, {low, l1}, {max, m1}, {high, h1}, {normal, n2},
               {max, m2}, {low, l2}],
    [begin
         spawn_opt(fun() -> Self ! {calling, Tag}, Value = Call(), Self ! {done, Tag, Value} end,
                   [{priority, Priority}]),
         receive {calling, Tag} -> ok end
     end || {Priority, Tag} <- Callers],
    Release(),
    Done = [receive {done, Tag, Value} -> {Value, Tag} end || {_, Tag} <- Callers],
    [Tag || {_, Tag} <- lists:sort(Done)].

%% The exception of a dirty call, with the first entry of its stack trace.
raise(Argument) ->
    try cpu_raise(Argument) catch Class:Reason:Stack -> {Class, Reason, hd(Stack)} end.

%% The run ends while a dirty call runs.
unfinished(Key) ->
    spawn(fun() -> cpu_wait(Key) end),
    await(Key).

%% Start one dirty CPU call for each key, and return their results.
waits(Keys) ->
    Self = self(),
    [spawn(fun() -> Self ! {Key, cpu_wait(Key)} end) || Key <- Keys],
    [receive {Key, Value} -> Value end || Key <- Keys].

%% The roots of a run on two schedulers: three dirty CPU calls in all.
first_root() -> waits([101, 102]).
second_root() -> waits([103]).

%% The roots of a run on two schedulers: the first root waits without a
%% timer for the message that the second root sends after its dirty call.
relay_wait() ->
    true = register(dirty_relay, self()),
    receive go -> ok end.

relay_send() ->
    Pid = wait_name(dirty_relay),
    ok = cpu_wait(201),
    Pid ! go,
    ok.

wait_name(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_name(Name) end;
        Pid -> Pid
    end.

await(Key) ->
    case started(Key) of
        true -> ok;
        false -> receive after 1 -> await(Key) end
    end.

%% Suppress the "nifs defined, but no call to erlang:load_nif/2" warning.
load() -> erlang:load_nif("dirty_nif_probe", 0).

cpu_thread() -> erlang:nif_error(undef).
io_thread() -> erlang:nif_error(undef).
cpu_wait(_Key) -> erlang:nif_error(undef).
started(_Key) -> erlang:nif_error(undef).
release(_Key) -> erlang:nif_error(undef).
cpu_raise(_Argument) -> erlang:nif_error(undef).
cpu_sequence() -> erlang:nif_error(undef).
