%% The table scenario of the benchmark of `benchmark/`.
%%
%% `insert_lookup/1` fills a `set` table with the objects `{Key, Key}` for the
%% keys 1 to N, reads each key again, and deletes the table. The benchmark
%% counts one object as one unit of work, so one unit holds one insert and one
%% lookup of a table that holds N objects.
-module(bench_ets).

-export([insert_lookup/1]).

insert_lookup(N) ->
    Table = ets:new(bench_ets, [set, private]),
    ok = insert(Table, 1, N),
    Sum = lookup(Table, 1, N, 0),
    true = ets:delete(Table),
    Sum.

insert(_Table, Key, N) when Key > N ->
    ok;
insert(Table, Key, N) ->
    true = ets:insert(Table, {Key, Key}),
    insert(Table, Key + 1, N).

lookup(_Table, Key, N, Acc) when Key > N ->
    Acc;
lookup(Table, Key, N, Acc) ->
    [{Key, Value}] = ets:lookup(Table, Key),
    lookup(Table, Key + 1, N, Acc + Value).
