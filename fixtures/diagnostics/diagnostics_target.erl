-module(diagnostics_target).
-export([id/1]).

%% Return the argument, so that the probe loads this module and can delete
%% its code.
id(Value) ->
    Value.
