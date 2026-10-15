-module(on_load_ok).
-export([value/0]).
-on_load(init/0).

%% The on_load function returns ok, so the code becomes current and the
%% exports become visible (code_server.erl:1360-1365).
init() ->
    ok.

value() ->
    on_load_ok_value.
