-module(bignum_key_probe).
-export([run/0, encoded/0, id/1]).

%% Map operations with bignum keys, in a flatmap and in a hashmap, alone and
%% inside lists and tuples. The fixture calls only BIFs, so it loads no
%% stdlib module. No key holds an atom: the hash of an atom depends on its
%% index in the atom table, which differs between ERTS and PBeam.
run() ->
    B = id(1 bsl 59),
    N = id(-(1 bsl 64) - 3),
    Keys = [B, N, B * B, 1, 1.0, {B}, [N], [1 | B], {7, [B, N]}],
    Pairs = [{K, v} || K <- Keys],
    Flat = id(maps:from_list(Pairs)),
    %% 40 keys: a hashmap with bignums of one, two and three digits.
    Big = [{id(1 bsl (59 + I * 4)) - I, I} || I <- seq(1, 31)],
    Hash = id(maps:from_list(Big ++ Pairs)),
    [maps:size(Flat), maps:keys(Flat), pairs(Flat),
     maps:size(Hash), maps:keys(Hash), pairs(Hash),
     map_get(B * B, Hash), map_get({7, [B, N]}, Hash), is_map_key(B + 1, Hash),
     maps:get((1 bsl 63) - 1, Hash, none), maps:get(1 bsl 63, Hash, none),
     maps:keys(maps:remove(B, Hash)), maps:keys(maps:remove(N, Flat)),
     maps:keys(Hash#{B := x, (B + 1) => y}),
     Flat =:= id(maps:from_list(reverse(Pairs))),
     Hash =:= id(maps:from_list(reverse(Big ++ Pairs))),
     #{B => 1} == #{float(B) => 1}, #{B => 1} =:= #{float(B) => 1},
     #{B => 1} < #{B + 1 => 1}, #{N => 1} < #{B => 1}].

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
