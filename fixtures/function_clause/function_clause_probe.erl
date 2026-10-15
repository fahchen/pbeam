-module(function_clause_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, pick/2, uncaught/0]).

%% The fixture has no line table, so ERTS gives each frame of this module
%% the location []. The location of a lists frame is [] on PBeam, so
%% stack/1 drops it. The frames of the erl -eval caller are dropped.
main() ->
    {[try_stack(fun() -> lists:nth(id(5), [a]) end),
      try_stack(fun() -> pick(id(x), 1) end),
      try_stack(fun() -> ?MODULE:pick(id(x), 2) end),
      try_stack(fun() -> body(id(x)) end),
      try_stack(fun() -> recurse(id(3), pick) end),
      try_stack(fun() -> recurse(id(3), error) end)],
     [catch_stack(fun() -> lists:nth(id(5), [a]) end),
      catch_stack(fun() -> pick(id(x), 1) end),
      catch_stack(fun() -> ?MODULE:pick(id(x), 2) end)],
     down()}.

id(X) -> X.

pick(a, N) -> N;
pick(b, N) when N > 0 -> N.

%% A body call keeps the frame of the caller in the stack trace.
body(X) ->
    Y = ?MODULE:pick(X, 3),
    id(Y).

%% ERTS records one frame for each run of equal continuation pointers, so
%% the frames of the recursion give one recurse/2 frame.
recurse(0, pick) -> pick(id(x), 5);
recurse(0, error) -> erlang:error(r, [a]);
recurse(N, How) ->
    V = recurse(N - 1, How),
    {V}.

uncaught() ->
    pick(id(x), 4).

try_stack(F) ->
    try F() catch C:R:S -> {C, R, stack(S)} end.

catch_stack(F) ->
    case catch F() of
        {'EXIT', {R, S}} -> {'EXIT', {R, stack(S)}}
    end.

stack([{M, F, A, _} | S]) when M =:= lists -> [{M, F, A, []} | stack(S)];
stack([{M, _, _, _} = Frame | S]) when M =:= ?MODULE -> [Frame | stack(S)];
stack(_) -> [].

%% An uncaught function_clause exits the process with
%% {function_clause, Stacktrace}.
down() ->
    {Pid, Ref} = spawn_monitor(?MODULE, uncaught, []),
    receive
        {'DOWN', Ref, process, Pid, Reason} -> Reason
    end.
