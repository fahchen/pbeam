%% A program whose module name another program also uses, with different code.
%% The test `rejects_a_local_fun_of_another_program` loads this module into one
%% program and the module of `fixtures/host_term_second` into another program.
-module(host_term_probe).

-export([created_atom/0, external_fun/0, local_fun/0, value/0]).

%% Return an atom that the run creates with `erlang:list_to_atom/1`. The name
%% is in the atom table of the run and not in the table of the program.
created_atom() ->
    list_to_atom("host_term_created").

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
