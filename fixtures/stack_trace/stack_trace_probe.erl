-module(stack_trace_probe).
-export([main/0, raise_uncaught/2]).

%% The fixture has no line table, so ERTS gives each frame of this module
%% the location []. main/0 sets backtrace_depth for the node, so it runs
%% alone in its own node.
main() ->
    Depth = [
        erlang:system_flag(backtrace_depth, 100),
        erlang:system_flag(backtrace_depth, (1 bsl 59) - 1),
        catch_class(fun() -> erlang:system_flag(backtrace_depth, id(-1)) end),
        catch_class(fun() -> erlang:system_flag(backtrace_depth, id(1 bsl 59)) end),
        erlang:system_flag(backtrace_depth, 1)
    ],
    Raise = [raise_frames(D) || D <- [0, 2, 8]],
    {Depth, Raise, raise_rethrow(), raise_down(error), raise_down(throw)}.

id(X) -> X.

catch_class(F) ->
    try F() catch C:R -> {C, R} end.

%% The class, the reason, and the stack trace that `try` gets. Each fun in
%% a frame becomes the atom 'fun', so that the result has no fun.
stack(F) ->
    try F() catch C:R:S -> {C, R, [funless(Frame) || Frame <- S]} end.

funless({F, Args, Location}) when is_function(F) -> {'fun', Args, Location};
funless(Frame) -> Frame.

frames() ->
    [{m, f, 1}, {m, f, [x], [{file, "m.erl"}, {line, 3}]}, {fun id/1, [x]}, {fun id/1, [x], [x]},
     {m, g, 2, []}].

%% erlang:raise/3 with each class, with the depth D. The first element is
%% the old depth. The frames with a location keep it, and the old frames
%% get the location [].
raise_frames(D) ->
    Old = erlang:system_flag(backtrace_depth, D),
    Frames = frames(),
    Long = [{m, f, N, []} || N <- lists:seq(1, 10)],
    [Old,
     stack(fun() -> erlang:raise(id(error), id(r), id(Frames)) end),
     stack(fun() -> erlang:raise(id(exit), id(r), id(Frames)) end),
     stack(fun() -> erlang:raise(id(throw), id(r), id(Frames)) end),
     stack(fun() -> erlang:raise(id(error), id(r), id(Long)) end),
     stack(fun() -> erlang:raise(id(error), id(r), id([{m, f, 1}])) end),
     stack(fun() -> erlang:raise(id(error), id(r), id([])) end)].

%% raw_raise raises the exception again with its frames.
raise_rethrow() ->
    stack(fun() ->
        try
            erlang:raise(id(exit), id(r), id(frames()))
        catch
            C:R:S -> erlang:raise(C, R, S)
        end
    end).

raise_uncaught(Class, Frames) ->
    erlang:raise(Class, r, Frames).

%% An uncaught error exits with {Reason, Stack}, and an uncaught throw with
%% {{nocatch, Reason}, Stack}.
raise_down(Class) ->
    {Pid, Ref} = spawn_monitor(?MODULE, raise_uncaught, [Class, [{m, f, 1}, {m, g, [], []}]]),
    receive
        {'DOWN', Ref, process, Pid, Reason} -> Reason
    end.
