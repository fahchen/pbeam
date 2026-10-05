-module(map_key_probe).
-export([list_keys/0, hash_keys/0, cons/2, id/1]).

list_keys() ->
    Big = ?MODULE:cons(300, [1]),
    Flat = ?MODULE:id(#{"b" => 1, a => 2, "a" => 3, [] => 4, [1, [2]] => 5, [a] => 6}),
    Hash = ?MODULE:id(#{"k01" => 0, "k02" => 1, "k03" => 2, "k04" => 3, "k05" => 4, "k06" => 5, "k07" => 6, "k08" => 7, "k09" => 8, "k10" => 9, "k11" => 10, "k12" => 11, "k13" => 12, "k14" => 13, "k15" => 14, "k16" => 15, "k17" => 16, "k18" => 17, "k19" => 18, "k20" => 19, "k21" => 20, "k22" => 21, "k23" => 22, "k24" => 23, "k25" => 24, "k26" => 25, "k27" => 26, "k28" => 27, "k29" => 28, "k30" => 29, default => 30, [[1], true] => 32, "\e[A" => 33}),
    #{"a" := A, [a] := B} = Flat,
    #{"\e[A" := C} = Hash,
    BigFlat = Flat#{Big => 7},
    BigHash = Hash#{Big => 31},
    [maps:keys(BigFlat), map_get(Big, BigFlat), A, B, maps:keys(Flat#{"b" := 8}),
     maps:keys(BigHash), map_get(Big, BigHash), C, map_get("k07", Hash#{"k07" := 34})].

hash_keys() ->
    Wide = ?MODULE:cons(256, [1, 2]),
    Negative = ?MODULE:cons(-1, []),
    Max = ?MODULE:cons(1 bsl 59 - 1, []),
    Min = ?MODULE:cons(-(1 bsl 59), []),
    maps:keys(#{[] => v, [[]] => v, [[[]]] => v, [1, 2, 3, 4, 5] => v, [1, 2, 3, 4, 5, 6, 7, 8, 9] => v, [true, [false], true] => v, [[1], [2]] => v, [1, true, 2, 3] => v, 0 => v, 1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, Wide => v, Negative => v, Max => v, Min => v}).

cons(Head, Tail) -> [Head | Tail].

id(Term) -> Term.
