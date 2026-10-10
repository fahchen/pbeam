-module(error_path_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, throw_reason/1]).

-record(r, {a}).

%% Each case raises an exception. `caught/1` gives the class and the reason
%% that `try` gets, and `caught_by_catch/1` gives the value of `catch`
%% without the stack trace, which PBeam does not record.
main() ->
    Cases = [
        fun() -> badmatch(id(b)) end,
        fun() -> badmatch_y(id(b)) end,
        fun() -> case_end(id(b)) end,
        fun() -> if_end(id(b)) end,
        fun() -> try_case_end(id(b)) end,
        fun() -> badrecord(id(b)) end,
        fun() -> raw_raise(id(error)) end,
        fun() -> raw_raise(id(exit)) end,
        fun() -> raw_raise(id(throw)) end,
        fun() -> erlang:error(id(r), [a]) end,
        fun() -> erlang:error(id(r), none) end,
        fun() -> erlang:throw(id(r)) end
    ],
    {[{caught(F), caught_by_catch(F)} || F <- Cases], raw_raise(id(foo)), down(), error_2_frames()}.

caught(F) ->
    try F() catch C:R -> {C, R} end.

caught_by_catch(F) ->
    case catch F() of
        {'EXIT', {Reason, Stack}} when is_list(Stack) -> {'EXIT', Reason};
        Value -> Value
    end.

id(X) -> X.

badmatch(X) ->
    {ok, _} = X,
    ok.

%% The value stays in a Y register across the call.
badmatch_y(X) ->
    V = id(X),
    id(V),
    {ok, _} = V,
    ok.

case_end(X) ->
    case X of
        a -> ok
    end.

if_end(X) ->
    if
        X =:= a -> ok
    end.

try_case_end(X) ->
    try id(X) of
        a -> ok
    catch
        _:_ -> caught
    end.

badrecord(X) ->
    X#r.a.

%% The stack trace is used only by erlang:raise/3, so the compiler emits
%% raw_raise. A class that is not error, exit, or throw gives badarg.
raw_raise(Class) ->
    try
        erlang:error(r)
    catch
        _:R:St -> erlang:raise(Class, R, St)
    end.

%% erlang:error/2 puts its arguments in the first frame of the stack trace,
%% or the arity of the caller when they are not a list. The location is
%% left out: PBeam has no line table.
error_2_frames() ->
    [first_frame(fun() -> error_2_tail(Args) end) || Args <- [[a], none, [a | b], []]] ++
        [first_frame(fun() -> error_2_body(id([a])) end)].

first_frame(F) ->
    try
        F()
    catch
        error:R:Stack ->
            [{M, Name, A, _Location} | _] = Stack,
            {R, M, Name, A}
    end.

error_2_tail(Args) ->
    erlang:error(id(r), Args).

error_2_body(Args) ->
    X = id(Args),
    erlang:error(r, X),
    id(X).

%% A throw with no catch ends the process with {{nocatch, Reason}, Stack}.
throw_reason(R) ->
    erlang:throw(R).

down() ->
    {Pid, Ref} = spawn_monitor(?MODULE, throw_reason, [r]),
    receive
        {'DOWN', Ref, process, Pid, {Reason, Stack}} when is_list(Stack) -> Reason
    end.
