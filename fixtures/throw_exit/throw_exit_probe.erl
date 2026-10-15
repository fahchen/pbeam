-module(throw_exit_probe).
-export([main/0, uncaught/0, root/1]).
-compile(nowarn_deprecated_catch).

%% The fixture has no line table, so ERTS gives each frame of this module
%% the location []. stack/1 drops the frames of the erl -eval caller.
main() ->
    [cases(throw), cases(exit), apply_initial()].

cases(Class) ->
    [try_stack(fun() -> raise(Class, r) end),
     try_stack(fun() -> tail(Class) end),
     try_stack(fun() -> fun_tail(fun_of(Class)) end),
     try_stack(fun() -> apply_tail(erlang, Class) end),
     catch_value(Class),
     initial_call(Class)].

id(X) -> X.

fun_of(throw) -> fun erlang:throw/1;
fun_of(exit) -> fun erlang:exit/1.

%% A body call of the BIF keeps the frame of the caller in the stack trace.
raise(throw, R) -> throw(R);
raise(exit, R) -> exit(R).

%% The compiler makes a call to the BIF in the last position a body call,
%% because the BIF does not return.
tail(throw) -> throw(t);
tail(exit) -> exit(t).

%% A tail call through a fun removes the frame of the caller first, and a
%% tail apply/3 keeps it (beam_common.c:1345-1431).
fun_tail(F) ->
    F(f).

apply_tail(M, F) ->
    apply(M, F, [a]).

catch_value(Class) ->
    catch raise(Class, c).

initial_call(Class) ->
    {_, Ref} = spawn_monitor(?MODULE, root, [fun_of(Class)]),
    receive {'DOWN', Ref, process, _, Reason} -> Reason end.

%% A process that starts at erlang:apply/3 runs its first apply with no
%% frame, so the error BIFs give an empty stack trace. A later tail apply of
%% the erlang.beam body keeps its frame (beam_common.c:1355-1366).
apply_initial() ->
    [down(erlang, apply, [erlang, throw, [x]]),
     down(erlang, apply, [erlang, apply, [erlang, throw, [x]]]),
     down(erlang, apply, [erlang, error, [x]]),
     down(erlang, apply, [erlang, apply, [fun erlang:apply/3, [erlang, error, [x]]]]),
     down(erlang, apply, [fun erlang:throw/1, [x]])].

down(M, F, A) ->
    {_, Ref} = spawn_monitor(M, F, A),
    receive {'DOWN', Ref, process, _, Reason} -> Reason end.

%% The fun call is the last call of the only frame of the process.
root(F) ->
    G = id(F),
    G(i).

%% The root process exits with the result of main/0, so the run gives its
%% text.
uncaught() ->
    exit(main()).

try_stack(F) ->
    try F() catch C:R:S -> {C, R, stack(S)} end.

stack([{M, _, _, _} = Frame | S]) when M =:= ?MODULE -> [Frame | stack(S)];
stack(_) -> [].
