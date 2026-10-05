-module(improper_probe).
-export([literals/0, id/1, ge/2, lt/2]).

%% Literal improper lists: exact equality, the term order, and map keys.
literals() ->
    Pairs = [
        {[1|x], [1,2]}, {[1|<<>>], [1,2]}, {[1,2|a], [1,2|b]}, {[1|a], [1]},
        {[1|2], [1|x]}, {[], [1|x]}, {{a}, [1|x]}, {[1|x], [1|x]}, {[1|{}], [1]}
    ],
    Flat = ?MODULE:id(#{[1|x] => a, [1] => b, x => c, [[1|2]|3] => d}),
    Hash = ?MODULE:id(#{[1|2] => v, [1,2|3] => v, [[1|2]|3] => v, [true|false] => v, [256|1] => v, [[1]|2] => v, [[]|-1] => v, 1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v}),
    {?MODULE:id([a|b]), ?MODULE:id({[1,2|{t}]}),
     [?MODULE:ge(A, B) || {A, B} <- Pairs], [?MODULE:lt(A, B) || {A, B} <- Pairs],
     [A =:= B || {A, B} <- Pairs], [A >= B || {A, B} <- Pairs],
     maps:keys(Flat), map_get([[1|2]|3], Flat), maps:keys(Hash), map_get([1,2|3], Hash)}.

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant term.
id(Term) -> Term.

%% The guard `A >= B` compiles to `is_ge`.
ge(A, B) when A >= B -> ge;
ge(_, _) -> lt.

%% The guard `A < B` compiles to `is_lt`.
lt(A, B) when A < B -> lt;
lt(_, _) -> ge.
