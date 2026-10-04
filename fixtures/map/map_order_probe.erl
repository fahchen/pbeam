-module(map_order_probe).
-export([main/0, flat32/1, hash40/1, add33/2, add40/2, mixed/1, negative/1, get40/1, keys/1, same/2]).
main() ->
    Flat32 = ?MODULE:flat32(v),
    Hash33 = ?MODULE:add33(Flat32, v),
    Hash40 = ?MODULE:hash40(v),
    Lit33 = #{1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v, 27 => v, 28 => v, 29 => v, 30 => v, 31 => v, 32 => v, 33 => v},
    {?MODULE:keys(Flat32),
     ?MODULE:keys(Hash33),
     ?MODULE:keys(Lit33),
     ?MODULE:same(Hash33, Lit33),
     ?MODULE:keys(Hash40),
     ?MODULE:same(?MODULE:add40(Hash33, v), Hash40),
     ?MODULE:same(Flat32, Hash33),
     ?MODULE:get40(Hash40),
     ?MODULE:get40(Hash33),
     ?MODULE:keys(?MODULE:mixed(v)),
     ?MODULE:keys(?MODULE:negative(v))}.
flat32(V) -> #{1 => V, 2 => V, 3 => V, 4 => V, 5 => V, 6 => V, 7 => V, 8 => V, 9 => V, 10 => V, 11 => V, 12 => V, 13 => V, 14 => V, 15 => V, 16 => V, 17 => V, 18 => V, 19 => V, 20 => V, 21 => V, 22 => V, 23 => V, 24 => V, 25 => V, 26 => V, 27 => V, 28 => V, 29 => V, 30 => V, 31 => V, 32 => V}.
hash40(V) -> #{1 => V, 2 => V, 3 => V, 4 => V, 5 => V, 6 => V, 7 => V, 8 => V, 9 => V, 10 => V, 11 => V, 12 => V, 13 => V, 14 => V, 15 => V, 16 => V, 17 => V, 18 => V, 19 => V, 20 => V, 21 => V, 22 => V, 23 => V, 24 => V, 25 => V, 26 => V, 27 => V, 28 => V, 29 => V, 30 => V, 31 => V, 32 => V, 33 => V, 34 => V, 35 => V, 36 => V, 37 => V, 38 => V, 39 => V, 40 => V}.
add33(M, V) -> M#{33 => V}.
add40(M, V) -> M#{34 => V, 35 => V, 36 => V, 37 => V, 38 => V, 39 => V, 40 => V}.
mixed(V) -> #{a => V, 2 => V, 1 => V}.
negative(V) -> #{5 => V, -1 => V, -101714049538469044 => V}.
get40(#{40 := V}) -> V;
get40(_) -> missing.
keys(M) -> maps:keys(M).
same(X, X) -> same;
same(_, _) -> different.
