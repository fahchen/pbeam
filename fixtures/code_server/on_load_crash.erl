-module(on_load_crash).
-export([value/0]).
-on_load(init/0).

%% The on_load function raises, so the loader process of code_server exits
%% with the reason (code_server.erl:1391-1394).
init() ->
    erlang:error(on_load_crash_reason).

value() ->
    on_load_crash_value.
