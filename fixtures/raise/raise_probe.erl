-module(raise_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, rethrow/1]).

main() ->
    {[{caught(Class), catch rethrow(Class)} || Class <- [error, exit, throw]], undef_frame(), down()}.

%% No catch clause matches the class of the exception, so the compiler
%% raises it again with the raise/2 instruction.
rethrow(error) ->
    try erlang:raise(error, {reason, error}, []) catch exit:_ -> exit; throw:_ -> throw end;
rethrow(exit) ->
    try erlang:raise(exit, {reason, exit}, []) catch error:_ -> error; throw:_ -> throw end;
rethrow(throw) ->
    try erlang:raise(throw, {reason, throw}, []) catch error:_ -> error; exit:_ -> exit end.

caught(Class) -> try rethrow(Class) catch C:R:S -> {C, R, S} end.

%% The stack trace of an undef exception starts with the frame of the
%% missing function, also after raise/2.
undef_frame() ->
    try
        try (id(?MODULE)):missing() catch exit:_ -> exit end
    catch
        error:R:S -> {R, hd(S)}
    end.

id(X) -> X.

%% A throw that raise/2 raises again with no catch ends the process with
%% {{nocatch, Reason}, Stacktrace}.
down() ->
    {Pid, Ref} = spawn_monitor(?MODULE, rethrow, [throw]),
    receive {'DOWN', Ref, process, Pid, Reason} -> Reason end.
