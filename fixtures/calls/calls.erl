-module(calls).
-export([main/0]).

main() -> external:value(local(7)).

local(Value) ->
    external:effect(Value),
    Value.
