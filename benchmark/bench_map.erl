%% The map scenario of the benchmark of `benchmark/`.
%%
%% `build_lookup/1` builds a map with the integer keys 1 to N, then reads each
%% key again with a tail-recursive loop. The benchmark counts one key as one
%% unit of work, so one unit holds one insert into the hash array mapped trie
%% and one lookup.
-module(bench_map).

-export([build_lookup/1]).

build_lookup(N) ->
    sum(1, N, build(N, #{}), 0).

build(0, Map) ->
    Map;
build(N, Map) ->
    build(N - 1, Map#{N => N}).

sum(Key, N, _Map, Acc) when Key > N ->
    Acc;
sum(Key, N, Map, Acc) ->
    sum(Key + 1, N, Map, Acc + maps:get(Key, Map)).
