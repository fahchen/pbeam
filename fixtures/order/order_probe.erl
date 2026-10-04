-module(order_probe).
-export([first/0, main/0, ge/2]).

%% `pbeam_order_z` gets a lower atom index than `pbeam_order_a`, so the
%% flatmap key order and the term order of the two atoms differ.
first() -> pbeam_order_z.

%% Each pair runs the guard `A >= B`, which compiles to `is_ge`.
main() ->
    Ref = make_ref(),
    Self = self(),
    Child = spawn(?MODULE, ge, [a, a]),
    Hash = #{1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v, 27 => v, 28 => v, 29 => v, 30 => v, 31 => v, 32 => v, 33 => v},
    HashW = #{1 => w, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v, 27 => v, 28 => v, 29 => v, 30 => v, 31 => v, 32 => v, 33 => v},
    Order = #{pbeam_order_z => 1, pbeam_order_a => 2},
    Pairs = [
        {1, a}, {a, 1}, {1, 2}, {1, 1},
        {b, a}, {ab, abc}, {'é', z}, {pbeam_order_z, pbeam_order_a},
        {Ref, a}, {Ref, Self}, {Self, Ref}, {{}, Self}, {Child, Self}, {Self, Child},
        {{b}, {a, a}}, {{a, b}, {a, a}}, {#{}, {a}},
        {#{a => 1}, #{}}, {#{z => 1}, #{a => 1, b => 1}}, {#{a => 2}, #{b => 1}},
        {#{a => 1, b => 2}, #{a => 2, b => 1}}, {#{1 => x}, #{a => x}},
        {Order, #{pbeam_order_z => 2, pbeam_order_a => 1}},
        {Hash, HashW}, {HashW, Hash}, {#{1 => v}, Hash},
        {[], #{}}, {[a], []}, {[a, b], [a, c]}, {[a, b], [a]}, {[b], [a, z]},
        {<<>>, [a]}, {<<1, 2>>, <<1, 3>>}, {<<1, 2>>, <<1>>}, {<<2>>, <<1, 9>>},
        {{a, [1, #{a => <<1>>}]}, {a, [1, #{a => <<1>>}]}}, {Ref, Ref}
    ],
    {maps:keys(Order), [?MODULE:ge(A, B) || {A, B} <- Pairs]}.

ge(A, B) when A >= B -> ge;
ge(_, _) -> lt.
