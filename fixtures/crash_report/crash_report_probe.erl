-module(crash_report_probe).
-export([error/0, throw/0, exit/0, no_logger/0, routes/0]).

%% Each function spawns a process that ends with an uncaught exception of
%% one class, waits for its exit, and gives logger time to write the report.
error() -> crash(fun() -> erlang:error({boom, fun lists:reverse/1}) end).

throw() -> crash(fun() -> erlang:throw(ball) end).

exit() -> crash(fun() -> erlang:exit(gone) end).

crash(Fun) ->
    {Pid, Ref} = spawn_monitor(Fun),
    receive {'DOWN', Ref, process, Pid, _} -> ok end,
    receive after 200 -> ok end,
    io:put_chars("done\n").

%% With no process registered as logger, ERTS writes the report message to
%% stderr (erl_trace.c:2564-2605).
no_logger() ->
    unregister(logger),
    error().

%% The routes of the system logger (erl_trace.c:2780-2824). A live pid gets
%% the message; undefined drops it; a pid that does not exist makes logger the
%% system logger again; logger sends it to the process registered as logger,
%% but not when that process is the sender. An uncaught exit sends nothing.
routes() ->
    Self = self(),
    Old = erlang:system_flag(system_logger, Self),
    Pid = wait(fun() -> erlang:error(boom) end),
    Live = receive
               {log, error, Format, [Pid, Reason], Metadata} ->
                   #{gl := Gl, pid := Pid, time := Time, error_logger := Tag} = Metadata,
                   {Format, Reason, maps:keys(Metadata), Tag, Gl =:= group_leader(),
                    is_integer(Time)}
           after 1000 -> none
           end,
    wait(fun() -> erlang:exit(boom) end),
    erlang:system_flag(system_logger, undefined),
    wait(fun() -> erlang:error(boom) end),
    Undefined = flush(),
    _ = [unregister(logger) || whereis(logger) =/= undefined],
    register(logger, Self),
    erlang:system_flag(system_logger, wait(fun() -> ok end)),
    Thrown = wait(fun() -> erlang:throw(ball) end),
    Dead = receive {log, error, _, [Thrown, R2], _} -> {erlang:system_info(system_logger), R2}
           after 1000 -> none
           end,
    unregister(logger),
    wait(fun() -> register(logger, self()), erlang:error(boom) end),
    Sender = flush(),
    erlang:system_flag(system_logger, Old),
    [Old, Live, Undefined, Dead, Sender].

wait(Fun) ->
    {Pid, Ref} = spawn_monitor(Fun),
    receive {'DOWN', Ref, process, Pid, _} -> Pid end.

flush() ->
    receive Message -> Message after 100 -> none end.
