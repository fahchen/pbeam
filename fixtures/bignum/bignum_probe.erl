-module(bignum_probe).
-export([literals/0, arith/0, compare/0, floats/0, text/0, bits/0, limits/0,
         splitmix/0, costs/0, segments/0, wide_segment/0]).

-define(REDS, element(2, process_info(self(), reductions))).
-define(TRY(Expr), try Expr catch error:Reason -> Reason end).
-define(MASK(Bits), ((1 bsl (Bits)) - 1)).
-define(BSL(Bits, X, N), (((X) band ?MASK((Bits) - (N))) bsl (N))).

%% Bignum literals: SMALL_BIG_EXT and LARGE_BIG_EXT in the literal table,
%% and integer operands of 9 or more bytes in the code.
literals() ->
    [[18446744073709551616, -18446744073709551616, 16#9e3779b97f4a7c15,
      -1606938044258990275541962092341162602522202993782792835301376],
     id(1) bsl 2100 =:= big_2100(),
     id(5) + 18446744073709551616,
     id(-5) - 340282366920938463463374607431768211456,
     case id(18446744073709551616) of 18446744073709551616 -> big; _ -> other end].

big_2100() ->
    1 bsl 2100.

%% `+`, `-`, `*`, `div`, and `rem` of bignums and small integers. A result
%% in the 64-bit range is a small integer term again.
arith() ->
    A = id(18446744073709551616), B = id(-340282366920938463463374607431768211457),
    Max = id(9223372036854775807), Min = id(-9223372036854775808),
    [A + 1, A + B, B - A, A * B, A * A, A div 7, B div A, B rem A, A rem 7,
     -7 rem A, A div B, (A - A) + 5, A - A =:= 0, (A * 2) div 2 - A,
     Max + 1, Min - 1, Max * 2, Min div -1, Min rem -1, -Min, abs(Min), abs(B), -A,
     id(-576460752303423488) div -1, A + 0, 0 + A, A - 0, A * 1, 1 * A, 0 * A].

%% Comparisons and the term order of bignums, small integers, and floats.
compare() ->
    A = id(18446744073709551616), F = id(18446744073709551616.0),
    Huge = id(1) bsl 1100,
    [A == F, A =:= F, A + 1 > F, A - 1 < F, A > 9223372036854775807, -A < -9223372036854775808,
     Huge > 1.0e300, -Huge < -1.0e300, A < 1.5e19 + 1.0e19, A > 1.5, -A < -1.5,
     sort([A, 1.0e19, F, -A, 3, 2.5, Huge, -Huge]),
     maps:keys(#{A => a, F => b, 1 => c}),
     lists:keyfind(A, 1, [{x, 1}, {F, found}]),
     lists:keyfind(F, 1, [{A, found}]),
     max(A, F), min(A, F), max(A, Huge), A =:= id(1) bsl 64,
     int(A), int(-Huge), int(F), is_float(A),
     is_integer(id(5), 0, A), is_integer(A, 0, A - 1), is_integer(Huge, -Huge, Huge),
     try is_integer(id(1), id(1.0), A) catch error:R -> R end].

%% Float conversion of bignums: the nearest float, ties to even, and badarg
%% or badarith for a bignum above the float range.
floats() ->
    A = id(18446744073709551616),
    [float(A), float(A + 1), float(A * 3 + 1), float(-(id(1) bsl 100)), A + 0.5, A / 2,
     A * 1.5, float((id(1) bsl 1023) * 3 div 2 + 1), ?TRY(float(id(1) bsl 1024)),
     ?TRY((id(1) bsl 1024) + 1.0), ?TRY((id(1) bsl 1024) / 2),
     float(id(16#1FFFFFFFFFFFFF) bsl 971)].

%% The text of bignums.
text() ->
    A = id(18446744073709551616),
    [integer_to_binary(A), integer_to_binary(-A * A), integer_to_list(A * 1000),
     integer_to_binary(A, 16), integer_to_binary(-A + 1, 36), integer_to_binary(-A, 2)].

%% Bitwise operations and shifts of bignums.
bits() ->
    A = id(18446744073709551616), M = id(16#FFFFFFFFFFFFFFFF), N = id(-340282366920938463463374607431768211457),
    [A band M, (A + 7) band M, N band M, N band A, A bor 5, N bor M, A bxor M, N bxor -1,
     bnot A, bnot N, bnot M, bnot id(1 bsl 60), id(3) bsl 58, A bsl 3, A bsr 3, N bsr 1, N bsr 100, N bsr 200, A bsr 200,
     id(1) bsl 64, id(-1) bsl 64, id(3) bsl 62, A bsl -64, id(5) bsl 0, A bsl 0,
     id(-576460752303423488) bsr 100, id(1) bsr (id(1) bsl 64), id(-1) bsr (id(1) bsl 64),
     id(1) bsl -(id(1) bsl 64), id(0) bsl (id(1) bsl 64),
     ?TRY(id(1) bsl (id(1) bsl 64)), ?TRY(id(1) bsr -(id(1) bsl 64)), ?TRY(id(a) bsl (id(1) bsl 64)),
     ?TRY(A bsl a), ?TRY(id(a) bsr 1), ?TRY(A band 1.0), ?TRY(bnot id(1.0)), ?TRY(A + a), ?TRY(A div 0),
     ?TRY(A div 1.0), ?TRY(A rem a), ?TRY(-id(a))].

%% Results above the largest bignum raise system_limit.
limits() ->
    One = id(1), Top = (One bsl 4194239) - 1 + (One bsl 4194239),
    [(One bsl 4194239) bsr 4194239, ?TRY(bnot Top), ?TRY(Top bxor id(-1)), (Top band id(-1)) bsr 4194239, ?TRY(One bsl 4194240), ?TRY(One bsl 100000000),
     ?TRY((One bsl 2100000) * (One bsl 2100000)),
     ?TRY((One bsl 4194239) + (One bsl 4194239))].

%% `splitmix64_next/1` and the scrambler of the default `rand` algorithm,
%% `exsss` (`rand.erl:2284-2303`, `rand.erl:3400-3404`), which the seed path of
%% `rand:seed/2` runs with bignums.
splitmix() ->
    {Z0, X0} = splitmix64_next(id(4711)),
    {Z1, X1} = splitmix64_next(X0),
    {Z2, _} = splitmix64_next(id(-1)),
    S0 = Z0 band ?MASK(58), S1 = Z1 band ?MASK(58),
    [Z0, X0, Z1, X1, Z2, exsss_next(S0, S1)].


splitmix64_next(X_0) ->
    X = (X_0 + 16#9e3779b97f4a7c15) band ?MASK(64),
    Z_0 = ((X bxor (X bsr 30)) * 16#bf58476d1ce4e5b9) band ?MASK(64),
    Z_1 = ((Z_0 bxor (Z_0 bsr 27)) * 16#94d049bb133111eb) band ?MASK(64),
    {(Z_1 bxor (Z_1 bsr 31)) band ?MASK(64), X}.

exsss_next(S0, S1) ->
    S1_b = (S1 band ?MASK(58)) bxor ?BSL(58, S1, 24),
    NewS1 = S1_b bxor S0 bxor (S1_b bsr 11) bxor (S0 bsr 41),
    V_a = S0 + ?BSL(58, S0, 2),
    V_b = ?BSL(58, V_a, 7) bor ((V_a bsr (58 - 7)) band ?MASK(7)),
    {(V_b + ?BSL(58, V_b, 3)) band ?MASK(58), NewS1}.

%% The reductions of bignum operations (bump_reds_linear and
%% bump_reds_quadratic in erl_arith.c:56-86).
costs() ->
    A = id(18446744073709551616), Small = id(5), Big = id(1) bsl 640, M = id(1 bsl 60), Big14 = id(1) bsl 832,
    [cost(fun() -> Small + Small end), cost(fun() -> A + Small end),
     cost(fun() -> A + 0 end), cost(fun() -> Big + Big end), cost(fun() -> A * A end),
     cost(fun() -> Big14 + 1 end), cost(fun() -> Big14 - 1 end), cost(fun() -> 1 - Big14 end),
     cost(fun() -> Big * Big end), cost(fun() -> A * 1 end), cost(fun() -> Big div A end),
     cost(fun() -> A div Big end), cost(fun() -> A rem 3 end), cost(fun() -> A bor 1 end),
     cost(fun() -> Big bxor 1 end), cost(fun() -> bnot A end), cost(fun() -> A bsl 1 end),
     cost(fun() -> Small bsl 64 end), cost(fun() -> Small bsl 3 end), cost(fun() -> Big bsr 1 end),
     cost(fun() -> bnot M end), cost(fun() -> -A end), cost(fun() -> -Big end), cost(fun() -> abs(Big) end),
     cost(fun() -> id(9223372036854775807) + Small end)].

cost(Fun) ->
    R0 = ?REDS,
    _ = Fun(),
    R1 = ?REDS,
    R1 - R0.

%% Binary segments of at most 64 bits keep the low bits of a bignum.
segments() ->
    A = id(18446744073709551616),
    [<<(A + 5):64>>, <<(-A - 1):8>>, <<(A + 16#0102):16/little>>].

%% A segment of more than 64 bits of a bignum.
wide_segment() ->
    <<(id(18446744073709551616) + 1):72>>.

int(X) when is_integer(X) -> true;
int(_) -> false.

%% An insertion sort in the term order, with `=<`.
sort(List) ->
    foldl(List, []).

foldl([], Sorted) -> Sorted;
foldl([X | Rest], Sorted) -> foldl(Rest, insert(X, Sorted)).

insert(X, [Y | Rest]) when Y =< X -> [Y | insert(X, Rest)];
insert(X, Sorted) -> [X | Sorted].

id(X) -> X.
