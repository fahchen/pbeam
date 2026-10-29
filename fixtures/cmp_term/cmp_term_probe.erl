-module(cmp_term_probe).
-export([pairs/0, id/1]).

%% `erts_internal:cmp_term/2` gives -1, 0, or 1 in the exact term order:
%% `CMP_TERM` calls `erts_cmp` with `exact = 1` (bif.c:5417-5427,
%% erl_utils.h:122), so each integer is less than each float, -0.0 is less
%% than 0.0, and each other pair follows the term order of data. `maps` reads
%% the same BIF to order two keys, so a Suikou `%Task{}` map with an alias
%% reference key depends on it (#1292).
pairs() ->
    Ref = make_ref(),
    Pairs = [{1, 1}, {1, 1.0}, {1.0, 1}, {1, 2}, {2, 1}, {1, a}, {a, 1}, {a, a}, {a, b},
             {-0.0, 0.0}, {0.0, -0.0}, {[], [1]}, {[1], []}, {{1, a}, {1, b}},
             {<<"a">>, <<"a">>}, {#{a => 1}, #{a => 1}}, {#{a => 1}, #{a => 2}},
             {Ref, Ref}, {576460752303423488, 576460752303423487}],
    [erts_internal:cmp_term(id(A), id(B)) || {A, B} <- Pairs].

id(X) -> X.
