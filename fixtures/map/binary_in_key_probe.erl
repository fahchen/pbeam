-module(binary_in_key_probe).
-export([run/0, encoded/0, id/1]).

%% Map operations with tuple, list, and map keys that hold a binary, in a
%% flatmap and in a hashmap. The binaries include a heap binary, a refc
%% binary of 100 bytes, a sub-binary, an unaligned sub-binary, and a
%% bitstring of 3 bits. The fixture calls only BIFs, so it loads no stdlib
%% module. No key holds an atom: the hash of an atom depends on its index in
%% the atom table, which differs between ERTS and PBeam.
run() ->
    Big = id(list_to_binary(seq(1, 100))),
    <<_:10/binary, Sub:50/binary, _/binary>> = Big,
    <<_:3, Odd:40/binary, _/bitstring>> = Big,
    <<Bits:3/bitstring, _/bitstring>> = id(<<255>>),
    Bins = [<<>>, <<"a">>, <<"ab">>, Big, Sub, Odd, Bits],
    Keys = [{<<"a">>, 1}, [<<"a">>], [1 | <<"a">>], {Big}, [Sub, Odd],
            #{1 => <<"a">>}, #{<<"b">> => 2}, {Bits}, [<<>>], {<<>>, <<>>},
            <<"a">>, Big],
    Pairs = [{K, v} || K <- Keys],
    Flat = id(maps:from_list(Pairs)),
    More = [{{B, I}, I} || I <- seq(1, 6), B <- Bins],
    Hash = id(maps:from_list(More ++ Pairs)),
    [maps:size(Flat), maps:keys(Flat), pairs(Flat),
     maps:size(Hash), maps:keys(Hash), pairs(Hash),
     map_get({id(<<"a">>), 1}, Hash), map_get({Sub, 3}, Hash),
     is_map_key({binary_part(Big, 10, 50), 3}, Hash),
     maps:get([id(<<"c">>)], Hash, none),
     maps:keys(maps:remove({Big}, Hash)),
     Flat =:= id(maps:from_list(reverse(Pairs))),
     Hash =:= id(maps:from_list(reverse(More ++ Pairs))),
     #{{<<"a">>} => 1} < #{{<<"b">>} => 1}, {Sub} == {binary_part(Big, 10, 50)}].

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
