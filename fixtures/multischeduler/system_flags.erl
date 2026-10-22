-module(system_flags).
-export([setter/0, reader/0, pair_reference/0]).

%% Run the pair of roots on the pinned ERTS, as PBeam runs the two roots on
%% two scheduler threads, and return the results.
pair_reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, setter()} end),
    _ = spawn(fun() -> Self ! {second, reader()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% The root that sets the node-wide flags, then waits until the root of the
%% other scheduler read and restored them. The start logger of ERTS is a pid
%% of the kernel, so the root returns only the old depth.
setter() ->
    Depth = erlang:system_flag(backtrace_depth, 3),
    _ = erlang:system_flag(system_logger, undefined),
    true = register(system_flags_setter, self()),
    receive
        restored -> Depth
    after 60000 -> timeout
    end.

%% The root that waits for the flags of the other root, reads them with the
%% stack trace depth and system_info/1, and restores them.
reader() ->
    Setter = wait_registered(1000000),
    Frames = length(deep_stacktrace(10)),
    Info = erlang:system_info(system_logger),
    Depth = erlang:system_flag(backtrace_depth, 8),
    Logger = erlang:system_flag(system_logger, logger),
    Setter ! restored,
    {Frames, Info, Depth, Logger}.

wait_registered(0) ->
    timeout;
wait_registered(N) ->
    case whereis(system_flags_setter) of
        undefined -> erlang:yield(), wait_registered(N - 1);
        Pid -> Pid
    end.

deep_stacktrace(N) ->
    try deep(N) catch error:_:Stacktrace -> Stacktrace end.

deep(0) -> error(deep);
deep(N) -> [N | deep(N - 1)].
