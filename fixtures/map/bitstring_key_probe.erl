-module(bitstring_key_probe).
-export([run/0, encoded/0, id/1]).

%% Map operations with top-level bitstring keys whose size is not a multiple
%% of 8 bits, in a flatmap and in a hashmap. The keys include bitstrings of
%% 1 to 403 bits, a sub-bitstring of a refc binary, and unaligned
%% sub-bitstrings. The fixture calls only BIFs, so it loads no stdlib module.
%% No key holds an atom: the hash of an atom depends on its index in the atom
%% table, which differs between ERTS and PBeam.
run() ->
    Big = id(list_to_binary(seq(1, 100))),
    <<_:5, Odd:13/bitstring, _/bitstring>> = Big,
    <<_:80, Sub:403/bitstring, _/bitstring>> = Big,
    <<_:3, Wide:61/bitstring, _/bitstring>> = Big,
    Bits = [<<1:1>>, <<0:1>>, <<1:3>>, <<5:7>>, <<1:9>>, <<255, 1:5>>,
            <<1, 2, 3:7>>, <<7:23>>, Odd, Sub, Wide],
    Pairs = [{B, v} || B <- Bits] ++ [{<<"a">>, 1}, {1, 2}],
    Flat = id(maps:from_list(Pairs)),
    More = [{<<I:6, B/bitstring>>, I} || I <- seq(1, 3), B <- Bits],
    Hash = id(maps:from_list(More ++ Pairs)),
    <<_:80, SubAgain:403/bitstring, _/bitstring>> = id(Big),
    [maps:size(Flat), maps:keys(Flat), pairs(Flat),
     maps:size(Hash), maps:keys(Hash), pairs(Hash),
     map_get(id(<<1:3>>), Flat), map_get(id(<<1:3>>), Hash),
     map_get(SubAgain, Hash), is_map_key(<<2:6, Odd/bitstring>>, Hash),
     maps:get(id(<<1:4>>), Hash, none),
     maps:get(id(<<1:3>>), id(#{<<1:2>> => x}), none),
     maps:keys(maps:remove(Wide, Hash)),
     Flat =:= id(maps:from_list(reverse(Pairs))),
     Hash =:= id(maps:from_list(reverse(More ++ Pairs)))].

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
