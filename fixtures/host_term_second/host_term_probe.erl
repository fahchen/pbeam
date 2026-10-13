%% The second program of the test `rejects_a_local_fun_of_another_program`.
%% It shares the module name of `fixtures/host_term_first` and holds different
%% code, so the entry label of a local fun of the first program names a
%% different function here.
-module(host_term_probe).

-export([accept/1, value/0]).

%% Return the fun argument.
accept(Fun) ->
    Fun.

%% Return the code marker of this program.
value() ->
    second.
