-module(ping_pong).
-export([ping/0, pong/0, reference/0, boom/0, wait/0, conflict_try/0, conflict_hold/0,
         monitor_watch/0, monitor_hold/0, unreg_hold/0, unreg_taker/0,
         info_watch/0, info_hold/0, info_item/0]).

-define(COUNT, 100).

%% The root of one scheduler. It waits for the other root, sends COUNT
%% messages in one burst, and returns whether the receiver saw them in order.
ping() ->
    true = register(ping, self()),
    Pong = wait_for(pong, 100000),
    % Cross the scheduler boundary with control signals before the messages.
    _ = link(Pong),
    true = unlink(Pong),
    _ = link(Pong),
    _ = monitor(process, Pong),
    _ = erlang:unique_integer(),
    _ = erlang:unique_integer([positive]),
    _ = erlang:unique_integer([monotonic]),
    _ = registered(),
    ok = send_pings(Pong, 1),
    Pong ! {done, self()},
    receive
        {done_ack, Next} -> {?COUNT, Next =:= ?COUNT + 1}
    after 60000 ->
        timeout
    end.

wait_for(_Name, 0) ->
    timeout;
wait_for(Name, Attempts) ->
    case whereis(Name) of
        undefined ->
            receive after 1 -> wait_for(Name, Attempts - 1) end;
        Pid ->
            Pid
    end.

send_pings(_Pid, N) when N > ?COUNT ->
    ok;
send_pings(Pid, N) ->
    Pid ! {ping, N},
    send_pings(Pid, N + 1).

%% The root of the other scheduler. It records the next expected sequence
%% number. A message out of order sets the state to -1, which the result
%% reports.
pong() ->
    true = register(pong, self()),
    pong_loop(1).

pong_loop(Next) ->
    receive
        {ping, N} when N =:= Next ->
            pong_loop(Next + 1);
        {ping, _N} ->
            pong_loop(-1);
        {done, From} ->
            From ! {done_ack, Next},
            ok
    after 60000 ->
        timeout
    end.

%% The reference entry for the pinned ERTS: one process for each root.
reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {ping, ping()} end),
    _ = spawn(fun() -> Self ! {pong, pong()} end),
    Ping = receive {ping, PingResult} -> PingResult after 60000 -> timeout end,
    Pong = receive {pong, PongResult} -> PongResult after 60000 -> timeout end,
    {Ping, Pong}.

%% A root that exits with an error.
boom() ->
    error(boom).

%% A root that waits for a message that no process can send.
wait() ->
    receive ok -> ok end.

%% The root that waits for the other root to hold the shared name, then finds
%% the name taken, stops the holder, and reports false.
conflict_try() ->
    ok = wait_registered(shared_name, 100000),
    false =
        try register(shared_name, self()) of
            _ -> unexpected
        catch
            error:badarg -> false
        end,
    case whereis(shared_name) of
        undefined -> ok;
        Pid -> Pid ! stop
    end,
    false.

%% The root that takes the shared name and keeps it until the other root
%% stops it.
conflict_hold() ->
    true = register(shared_name, self()),
    receive stop -> true after 60000 -> timeout end.

wait_registered(_Name, 0) ->
    timeout;
wait_registered(Name, Attempts) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_registered(Name, Attempts - 1) end;
        _Pid -> ok
    end.

%% The root that monitors the other root while it waits for the monitor to
%% leave the pending slot, then reports the reason of the DOWN message.
monitor_watch() ->
    Pid = wait_for(monitor_hold_name, 100000),
    Ref = monitor(process, Pid),
    receive
        {'DOWN', Ref, process, Pid, normal} -> true;
        {'DOWN', Ref, process, Pid, Other} -> Other
    after 60000 -> timeout
    end.

%% The root that exits normally after a short wait.
monitor_hold() ->
    true = register(monitor_hold_name, self()),
    receive after 10 -> ok end.

%% The root that holds a name until the other root unregisters it, then
%% registers the name again.
unreg_hold() ->
    true = register(unreg_name, self()),
    ok = wait_cleared(20000),
    true = register(unreg_name, self()),
    true.

wait_cleared(0) ->
    timeout;
wait_cleared(Attempts) ->
    case process_info(self(), registered_name) of
        [] ->
            ok;
        _ ->
            receive after 1 -> wait_cleared(Attempts - 1) end
    end.

%% The root that unregisters the name of the other root.
unreg_taker() ->
    ok = wait_registered(unreg_name, 20000),
    true = unregister(unreg_name),
    true.

%% The root that reads the registered name and the liveness of a process of
%% another scheduler, and then stops it.
info_watch() ->
    Pid = wait_for(info_name, 100000),
    {registered_name, info_name} = process_info(Pid, registered_name),
    true = is_process_alive(Pid),
    Pid ! stop,
    ok.

%% The root that holds the name of the other root.
info_hold() ->
    true = register(info_name, self()),
    receive stop -> ok after 60000 -> timeout end.

%% The root that asks for an item of a remote process that PBeam cannot read
%% yet. The run stops with the unsupported error.
info_item() ->
    Pid = wait_for(info_name, 100000),
    _ = process_info(Pid, message_queue_len),
    Pid ! stop,
    ok.
