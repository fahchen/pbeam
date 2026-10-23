-module(bs_create_bin_bits_probe).
-export([nibble/1, bits/2, unit3/1, sized/2, sized_bits/2, append/2, acc/1, acc_bytes/1, tail/2, cases/0]).

%% Bitstrings that `bs_create_bin` builds, with a size that is not a
%% multiple of 8 bits.
nibble(X) -> <<X:4>>.
bits(A, B) -> <<A/bits, B/bitstring>>.
unit3(A) -> <<A/binary-unit:3>>.
sized(A, N) -> <<A:N/binary>>.
sized_bits(A, N) -> <<A:N/bits, 1:1>>.
append(A, B) -> <<A/bits, B:3>>.

%% Loops that the compiler gives a `private_append` segment.
acc(N) -> acc(N, <<>>).
acc(0, Acc) -> Acc;
acc(N, Acc) -> acc(N - 1, <<Acc/bits, N:1>>).
acc_bytes(N) -> acc_bytes(N, <<1:4>>).
acc_bytes(0, Acc) -> Acc;
acc_bytes(N, Acc) -> acc_bytes(N - 1, <<Acc/bits, N:8>>).

tail(A, B) -> <<A/bits, B/binary>>.

%% Exit with the result of each case, so that the exit reason shows the
%% terms in the form of `~w`.
cases() ->
    exit([show(F, Args) || {F, Args} <- [
        {nibble, [1]}, {nibble, [-1]},
        {bits, [<<1:3>>, <<5:5, 1:2>>]}, {bits, [<<>>, <<1:7>>]},
        {unit3, [<<1:6>>]}, {unit3, [<<1:4>>]}, {unit3, [a]},
        {sized, [<<1, 2, 3>>, 2]}, {sized, [<<1, 2>>, 3]}, {sized, [<<1, 2>>, -1]},
        {sized, [<<1, 2>>, a]}, {sized, [a, 1]}, {sized, [a, a]}, {sized, [<<1:4, 2>>, 1]},
        {sized, [<<1>>, 1 bsl 70]}, {sized, [<<1>>, -(1 bsl 70)]}, {sized, [<<1:4>>, 0]},
        {sized_bits, [<<255>>, 3]}, {sized_bits, [<<1:3>>, 4]}, {sized_bits, [a, 1]},
        {append, [<<1:5>>, 7]}, {append, [<<1, 2>>, 7]}, {append, [a, 7]},
        {acc, [5]}, {acc, [0]}, {acc, [17]}, {acc_bytes, [2]},
        {tail, [<<1:1>>, <<255>>]}, {tail, [<<1:1>>, <<1:1>>]}
    ]]).

%% Return the result of `F(Args...)`, or the class, the reason, and the
%% `error_info` cause of its exception.
show(F, Args) ->
    try apply(?MODULE, F, Args)
    catch C:R:S ->
        [{_, _, _, I} | _] = S,
        {C, R, cause(I)}
    end.

cause([{error_info, #{cause := Cause}} | _]) -> Cause;
cause([_ | Info]) -> cause(Info).
