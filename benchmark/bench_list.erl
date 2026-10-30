%% The list scenario of the benchmark of `benchmark/`.
%%
%% `build_sum/1` builds a list of the integers 1 to N, then sums it with a
%% tail-recursive loop. The benchmark counts one list cell as one unit of
%% work, so one unit holds the allocation of the cell and its later read.
-module(bench_list).

-export([build_sum/1, build/2, sum/2]).

build_sum(N) ->
    sum(build(N, []), 0).

build(0, Acc) ->
    Acc;
build(N, Acc) ->
    build(N - 1, [N | Acc]).

sum([], Acc) ->
    Acc;
sum([Head | Tail], Acc) ->
    sum(Tail, Head + Acc).
