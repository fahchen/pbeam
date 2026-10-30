%% The arithmetic scenario of the benchmark of `benchmark/`.
%%
%% `loop/1` runs a tail-recursive loop with a comparison, four integer
%% operations, and one local call for each iteration. The accumulator stays
%% below 2^32, so each operation is a small-integer BIF and no operation
%% allocates a bignum. The benchmark counts one loop iteration as one unit of
%% work.
-module(bench_arith).

-export([loop/1]).

loop(0) ->
    0;
loop(N) ->
    loop(N, 0).

loop(0, Acc) ->
    Acc;
loop(N, Acc) ->
    loop(N - 1, (Acc * 33 + N) band 16#FFFFFFFF).
