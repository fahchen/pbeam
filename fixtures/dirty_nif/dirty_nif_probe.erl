-module(dirty_nif_probe).
-export([threads/0, signals/1, signals_with/3, kill/1, kill_with/3, queued/1,
         raise/1, unfinished/1, waits/1, first_root/0, second_root/0, load/0]).

-nifs([cpu_thread/0, io_thread/0, cpu_wait/1, started/1, release/1, cpu_raise/1]).

%% The test registers each NIF of this module as a Rust native function:
%% `cpu_thread/0`, `cpu_wait/1`, and `cpu_raise/1` with the job `CpuBound`,
%% `io_thread/0` with the job `IoBound`, and `started/1` and `release/1`
%% with the job `Normal`. `cpu_wait(Key)` marks `Key` as started, waits until
%% `release(Key)`, and returns `ok`, as `erts_debug:dirty_cpu(wait, Ms)`
%% returns `ok` after `Ms` milliseconds on ERTS.

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
