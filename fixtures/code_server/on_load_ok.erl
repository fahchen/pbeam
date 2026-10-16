-module(on_load_ok).
-export([value/0]).
-on_load(init/0).

%% The on_load function returns ok, so the code becomes current and the
%% exports become visible (code_server.erl:1360-1365). It calls a fun of
%% the module, which runs while the exports are hidden. The fun goes
%% through the process dictionary, so that the compiler keeps the fun call.
init() ->
    put(on_load_ok_fun, fun(Value) -> Value end),
    Fun = erase(on_load_ok_fun),
    Fun(ok).

value() ->
    on_load_ok_value.
