-module(undef_probe).
-compile(nowarn_deprecated_catch).
-export([calls/0, down/1, id/1, last/1, only/1, apply_last/3, apply3_last/3]).

%% Call functions that undef_target and undef_handler do not export, through
%% each call instruction. A try returns the class, the reason, and the first
%% frame of the stack trace: ERTS puts the frames of the callers after it
%% (error_handler:crash/1, error_handler.erl:172-178). undef_handler exports
%% '$handle_undefined_function'/2, so error_handler calls it instead
%% (error_handler.erl:206-213).
calls() ->
    M = id(undef_target),
    F = id(missing),
    [try undef_target:missing(a) catch C1:R1:S1 -> first(C1, R1, S1) end,
     try ?MODULE:last(b) catch C2:R2:S2 -> first(C2, R2, S2) end,
     try ?MODULE:only(c) catch C3:R3:S3 -> first(C3, R3, S3) end,
     try M:F(d) catch C4:R4:S4 -> first(C4, R4, S4) end,
     try ?MODULE:apply_last(M, F, e) catch C5:R5:S5 -> first(C5, R5, S5) end,
     try erlang:apply(M, F, id([f])) catch C6:R6:S6 -> first(C6, R6, S6) end,
     try ?MODULE:apply3_last(M, F, [g]) catch C7:R7:S7 -> first(C7, R7, S7) end,
     try erlang:apply(id(fun undef_target:missing/1), id([h])) catch C8:R8:S8 -> first(C8, R8, S8) end,
     try (id(fun undef_target:missing/1))(i) catch C9:R9:S9 -> first(C9, R9, S9) end,
     try (erlang:make_fun(M, F, 2))(j, k) catch C10:R10:S10 -> first(C10, R10, S10) end,
     try undef_target:missing() catch C11:R11:S11 -> first(C11, R11, S11) end,
     case catch undef_target:missing(l) of {'EXIT', {R12, [Frame | _]}} -> {'EXIT', R12, Frame} end,
     undef_handler:missing(m),
     erlang:apply(undef_handler, id(other), [n, o])].

%% Return the DOWN reason of a process that starts at a function that the
%% module does not export.
down(Module) ->
    {Pid, Ref} = spawn_monitor(Module, missing, [p]),
    receive {'DOWN', Ref, process, Pid, Reason} -> Reason end.

id(X) -> X.

first(Class, Reason, [Frame | _]) -> {Class, Reason, Frame}.

last(X) ->
    id(x),
    undef_target:missing(X).

only(X) -> undef_target:missing(X).

apply_last(M, F, X) ->
    id(x),
    M:F(X).

apply3_last(M, F, A) ->
    id(x),
    erlang:apply(M, F, A).
