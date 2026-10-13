%% A program whose module name another program also uses, with different code.
%% The test `rejects_a_local_fun_of_another_program` loads this module into one
%% program and the module of `fixtures/host_term_second` into another program.
-module(host_term_probe).

-export([created_atom/1, external_fun/0, local_fun/0, value/0]).

%% Return the atom of the character list `chars`. The caller makes the list
%% at run time, so the name is not in the atom table of the program and only
%% the run that calls this function creates it.
created_atom(Chars) ->
    list_to_atom(Chars).

%% Return a local fun of this module. The fun holds the entry label of
%% `value/0` in this program.
local_fun() ->
    fun value/0.

%% Return an external fun of this module and function. The fun holds no entry
%% label and resolves through the export of the destination program.
external_fun() ->
    fun host_term_probe:value/0.

%% Return the code marker of this program.
value() ->
    first.
