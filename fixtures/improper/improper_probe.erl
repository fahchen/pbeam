-module(improper_probe).
-export([literals/0, instructions/0, cons/2, id/1, ge/2, lt/2]).

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

%% put_list, get_list, get_hd, get_tl, is_list, and is_nonempty_list.
instructions() ->
    Ref = make_ref(),
    Tag = ?MODULE:cons(alias, Ref),
    self() ! {Tag, reply},
    Reply = receive {[alias | Mref], R} when Mref =:= Ref -> R end,
    Imp = ?MODULE:cons(1, x),
    Two = ?MODULE:cons(0, Imp),
    Nested = ?MODULE:cons(?MODULE:cons(1, 2), 3),
    [Reply, Imp, Two, Nested, head(Two), tail(Two), tail(Imp), split(Nested),
     kind(Imp), kind(?MODULE:id([])), kind(?MODULE:id(x)), ?MODULE:cons(Two, []), Tag =:= [alias | Ref]].

head([H | _]) -> H.
tail([_ | T]) -> T.
split([[A | B] | C]) -> {A, B, C}.
kind(L) when is_list(L), L =/= [] -> nonempty;
kind([]) -> nil;
kind(_) -> other.

%% Return a list cell with a head and a tail. The tail of `gen:do_call/4`
%% is a reference, so `put_list` gets a register tail.
cons(Head, Tail) -> [Head | Tail].

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant term.
id(Term) -> Term.

%% The guard `A >= B` compiles to `is_ge`.
ge(A, B) when A >= B -> ge;
ge(_, _) -> lt.

%% The guard `A < B` compiles to `is_lt`.
lt(A, B) when A < B -> lt;
lt(_, _) -> ge.
