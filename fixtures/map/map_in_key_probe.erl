-module(map_in_key_probe).
-export([run/0, encoded/0, id/1]).

%% Map operations with keys that are maps or hold a map, in a flatmap and in
%% a hashmap, with flatmap and hashmap keys. The fixture calls only BIFs, so
%% it loads no stdlib module. No key holds an atom: the hash of an atom
%% depends on its index in the atom table, which differs between ERTS and
%% PBeam.
run() ->
    E = id(#{}),
    F = id(#{1 => 2, 3.0 => [4], {5} => 1 bsl 64}),
    G = id(#{1 => #{2 => 3}}),
    %% A hashmap of 40 keys as a key, and a copy with one more key.
    H = id(maps:from_list([{I, -I} || I <- seq(1, 40)])),
    H2 = id(H#{41 => 0}),
    Keys = [E, F, G, H, H2, {E}, [F, 1], [1 | G], {1, [H]}, #{E => E}, 1, 1.0],
    Pairs = [{K, v} || K <- Keys],
    Flat = id(maps:from_list(Pairs)),
    %% 40 more keys: a hashmap of map keys.
    More = [{#{I => I}, I} || I <- seq(1, 40)],
    Hash = id(maps:from_list(More ++ Pairs)),
    [maps:size(Flat), maps:keys(Flat), pairs(Flat),
     maps:size(Hash), maps:keys(Hash), pairs(Hash),
     map_get(H, Hash), map_get({1, [H]}, Hash), is_map_key(#{1 => 2.0}, Hash),
     maps:get(#{1 => 1.0}, Hash, none), maps:get(#{7 => 7}, Hash, none),
     maps:keys(maps:remove(G, Hash)), maps:keys(maps:remove(E, Flat)),
     maps:keys(Hash#{F := x, #{1 => 3} => y}),
     Flat =:= id(maps:from_list(reverse(Pairs))),
     Hash =:= id(maps:from_list(reverse(More ++ Pairs))),
     #{E => 1} == #{#{} => 1}, #{G => 1} < #{F => 1}, #{H => 1} < #{H2 => 1}].

%% The external term format of the result, which also holds each map.
encoded() -> term_to_binary(run()).

%% The pairs of a map in map order, as maps:to_list/1 builds them
%% (maps.erl:525-542).
pairs(Map) -> collect(erts_internal:map_next(0, Map, [])).

collect([Iter, Map | Acc]) when is_integer(Iter) ->
    collect(erts_internal:map_next(Iter, Map, Acc));
collect(Acc) -> Acc.

seq(First, Last) when First > Last -> [];
seq(First, Last) -> [First | seq(First + 1, Last)].

reverse(List) -> reverse(List, []).
reverse([], Acc) -> Acc;
reverse([H | T], Acc) -> reverse(T, [H | Acc]).

id(Term) -> Term.
