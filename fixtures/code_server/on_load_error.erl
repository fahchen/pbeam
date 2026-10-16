-module(on_load_error).
-export([value/0]).
-on_load(init/0).

%% The on_load function returns a value that is not ok, so code_server
%% removes the code (code_server.erl:1360-1368).
init() ->
    {error, on_load_error_reason}.

value() ->
    on_load_error_value.
