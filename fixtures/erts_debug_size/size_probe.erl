-module(size_probe).
-export([sizes/0, show/0, hashmap/0, binary_type/0, id/1]).

%% Return erts_debug:size/1 of a table of shared and unshared terms of
%% each kind, then erts_debug:same/2 and erts_internal:term_type/1 cases.
sizes() ->
    X = id(7),
    A = id([a, b]),
    T = {a, X},
    F = id(1.5),
    B = id(1 bsl 70),
    M = #{1 => A, 2 => A},
    Fun = fun() -> {A, X} end,
    Terms = [1, a, [], self(), F, {F, F}, B, {B, B}, make_ref(),
             <<1, 2, 3>>, binary:copy(<<"a">>, id(100)), {}, T, {T, T},
             A, [A, A], [a | b], #{}, #{a => 1}, M, {M, M}, Fun,
             {Fun, Fun}, fun lists:map/2, [Fun | A]],
    Sizes = [erts_debug:size(Term) || Term <- id(Terms)],
    Same = [erts_debug:same(A, A), erts_debug:same(A, id([a, b])),
            erts_debug:same(1, 1), erts_debug:same(a, b),
            erts_debug:same(T, T), erts_debug:same(M, M),
            erts_debug:same(Fun, Fun), erts_debug:same(B, B)],
    Types = [erts_internal:term_type(Term)
             || Term <- id([X, 1 bsl 59, B, F, a, [], A, T, M,
                            maps:from_list([{I, I} || I <- lists:seq(1, id(33))]),
                            Fun, fun lists:map/2, self(), make_ref()])],
    Keys = [erts_internal:map_to_tuple_keys(Map) || Map <- id([#{}, M])],
    Errors = [error_of(a),
              error_of(maps:from_list([{I, I} || I <- lists:seq(1, id(33))]))],
    {Sizes, Same, Types, Keys, Errors}.

%% Exit with the result of sizes/0, so the host reads it.
show() ->
    exit(sizes()).

%% Return erts_debug:size/1 of a map of more than 32 keys.
hashmap() ->
    erts_debug:size(maps:from_list([{I, I} || I <- lists:seq(1, id(33))])).

%% Return erts_internal:term_type/1 of a binary.
binary_type() ->
    erts_internal:term_type(id(<<1, 2, 3>>)).

error_of(Term) ->
    try erts_internal:map_to_tuple_keys(Term)
    catch Class:Reason -> {Class, Reason}
    end.

id(Term) ->
    Term.
