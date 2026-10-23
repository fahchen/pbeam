-module(fun_key_probe).
-export([flat/0, hash/0, nested/0, order/0, external/0, id/1]).

%% The value of each map operation with a fun key. Each result holds no fun,
%% so a run on the pinned ERTS and a run on PBeam print the same text.
mk(N) -> fun() -> N end.

flat() ->
    F1 = id(mk(1)), F2 = id(mk(2)), G = id(fun(X) -> X end),
    M = id(#{F1 => 1, F2 => 2, G => 3, {F1} => 4, [F2] => 5}),
    [maps:size(M), map_get(F1, M), map_get(mk(2), M), map_get(G, M),
     map_get({mk(1)}, M), map_get([F2], M), is_map_key(mk(3), M),
     [V || {_, V} <- maps:to_list(M)]].

%% A hashmap of 40 local fun keys: the pair order follows the internal hash.
hash() ->
    M = id(maps:from_list([{mk(N), N} || N <- lists:seq(1, 40)])),
    [maps:size(M), map_get(mk(17), M), is_map_key(mk(41), M),
     [F() || {F, _} <- maps:to_list(M)]].

%% A hashmap of fun keys inside tuples and lists.
nested() ->
    M = id(maps:from_list([{{N, mk(N)}, N} || N <- lists:seq(1, 34)]
                          ++ [{[mk(N)], -N} || N <- lists:seq(1, 4)])),
    [maps:size(M), map_get([mk(2)], M), maps:values(M)].

%% The term order of local funs.
order() ->
    [mk(1) < mk(2), mk(2) < mk(1), mk(1) =:= mk(1), mk(1) < fun(X) -> X end,
     lists:sort([mk(3), mk(1), mk(2)]) =:= [mk(1), mk(2), mk(3)]].

%% External fun keys of a flatmap.
external() ->
    F = id(fun lists:reverse/1), G = id(fun lists:sort/1),
    M = id(#{F => 1, G => 2, mk(1) => 3}),
    [map_get(fun lists:reverse/1, M), map_get(G, M),
     [V || {_, V} <- maps:to_list(M)]].

id(T) -> T.
