-module(diagnostics_on_load).
-on_load(init/0).
-export([id/1]).

%% The on_load function, so that erlang:has_prepared_code_on_load/1 returns
%% true for the prepared code of this module.
init() ->
    ok.

%% Return the argument. The probe prepares the code but never loads it.
id(Value) ->
    Value.
