-module(maps_fun_probe).
-export([main/0, tail/3]).

%% `maps:get/2` and `maps:is_key/2` through a fun call and through
%% `apply/3` with a module atom that the compiler cannot see. ERTS runs the
%% `maps_get_2` and `maps_is_key_2` BIFs (`erl_map.c:250-264`,
%% `erl_map.c:1185-1191`). Each error gives the reason and the top frame
%% of the stack trace with its `error_info`.
main() ->
    Maps = binary_to_atom(id(<<"maps">>)),
    Get = id(fun maps:get/2),
    IsKey = id(fun maps:is_key/2),
    Map = id(#{a => 1}),
    [Get(a, Map), IsKey(a, Map), IsKey(b, Map),
     apply(Maps, get, [a, Map]), apply(Maps, is_key, [b, Map]),
     caught(fun() -> Get(b, Map) end),
     caught(fun() -> Get(a, id(x)) end),
     caught(fun() -> IsKey(a, id(x)) end),
     caught(fun() -> apply(Maps, get, [b, Map]) end),
     caught(fun() -> apply(Maps, is_key, [a, id(x)]) end),
     tail(Maps, get, [a, Map]), caught(fun() -> tail(Maps, get, [b, Map]) end)].

%% A tail call of `apply/3`, which the compiler emits as `apply_last`.
tail(Module, Name, Arguments) -> apply(Module, Name, Arguments).

caught(Fun) ->
    try Fun() catch error:Reason:Stack -> {Reason, hd(Stack)} end.

id(Term) -> Term.
