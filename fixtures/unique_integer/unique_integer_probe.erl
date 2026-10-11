-module(unique_integer_probe).
-export([main/0, child/1]).

%% Three erlang:unique_integer/0 values of this process, then one of a new
%% process. All processes of a scheduler share its counter
%% (erl_bif_unique.c:565-578).
main() ->
    Own = [erlang:unique_integer() || _ <- [1, 2, 3]],
    spawn(?MODULE, child, [self()]),
    receive {unique, Value} -> Own ++ [Value] end.

child(Parent) ->
    Parent ! {unique, erlang:unique_integer()}.
