-module(code_server_target).
-export([value/0]).

%% The module that code_server_probe loads through code:ensure_loaded/1. It
%% is on the code path and not in the start set of the run.
value() -> code_server_target_value.
