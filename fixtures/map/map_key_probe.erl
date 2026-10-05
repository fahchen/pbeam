-module(map_key_probe).
-export([list_keys/0, hash_keys/0, collision_keys/0, cons/2, id/1]).

list_keys() ->
    Flat = ?MODULE:id(#{"b" => 1, a => 2, "a" => 3, [] => 4, [1, [2]] => 5, [a] => 6, [300] => 7}),
    Hash = ?MODULE:id(#{"k01" => 0, "k02" => 1, "k03" => 2, "k04" => 3, "k05" => 4, "k06" => 5, "k07" => 6, "k08" => 7, "k09" => 8, "k10" => 9, "k11" => 10, "k12" => 11, "k13" => 12, "k14" => 13, "k15" => 14, "k16" => 15, "k17" => 16, "k18" => 17, "k19" => 18, "k20" => 19, "k21" => 20, "k22" => 21, "k23" => 22, "k24" => 23, "k25" => 24, "k26" => 25, "k27" => 26, "k28" => 27, "k29" => 28, "k30" => 29, default => 30, [300, 1] => 31, [[1], true] => 32, "\e[A" => 33}),
    Big = ?MODULE:cons(300, [1]),
    #{"a" := A, [300] := B} = Flat,
    #{"\e[A" := C, Big := D} = Hash,
    [maps:keys(Flat), map_get("a", Flat), maps:keys(Hash), map_get([300, 1], Hash),
     A, B, C, D, maps:keys(Flat#{Big => 8}), map_get("b", Flat#{"b" := 9}),
     map_get(Big, Hash#{Big => 34}), map_get("k07", Hash#{"k07" := 35})].

hash_keys() ->
    Wide = ?MODULE:cons(256, [1, 2]),
    Negative = ?MODULE:cons(-1, []),
    Max = ?MODULE:cons(1 bsl 59 - 1, []),
    Min = ?MODULE:cons(-(1 bsl 59), []),
    maps:keys(#{[] => v, [[]] => v, [[[]]] => v, [1, 2, 3, 4, 5] => v, [1, 2, 3, 4, 5, 6, 7, 8, 9] => v, [true, [false], true] => v, [[1], [2]] => v, [1, true, 2, 3] => v, 0 => v, 1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, Wide => v, Negative => v, Max => v, Min => v}).

collision_keys() ->
    A = [246, 157, 203, 77, 19, 254, 19, 93],
    B = [161, 92, 65, 50, 231, 24, 153, 137],
    Hash = ?MODULE:id(#{A => a, 1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v, 27 => v, 28 => v, 29 => v, 30 => v, 31 => v, 32 => v}),
    Both = Hash#{B => b},
    Literal = #{A => a, B => b, 1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v, 27 => v, 28 => v, 29 => v, 30 => v, 31 => v, 32 => v},
    #{A := X, B := Y} = Both,
    [maps:keys(Both), X, Y, map_get(B, Both), Both =:= Literal, map_get(A, Both#{A := c}), map_get(B, Both#{B => d}), maps:keys(#{A => 1, B => 2})].

cons(Head, Tail) -> [Head | Tail].

id(Term) -> Term.
