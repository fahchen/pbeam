-module(tuple_key_probe).
-export([main/0, grid/0, errors/0, id/1]).

main() ->
    Flat = ?MODULE:id(#{{1, 2} => a, {0, 5} => b, {1} => c, {value} => d, {abs} => e, value => f, abs => g, 3 => h, [1] => i, {} => j, {[1], {2}} => k}),
    #{{1, 2} := A, {value} := D} = Flat,
    Key = ?MODULE:id({2, 1}),
    Assoc = Flat#{Key => l, {1, 2} => m},
    Exact = (?MODULE:id(Assoc))#{{0, 5} := n},
    [maps:keys(Flat), A, D, map_get({1}, Flat), maps:get({}, Flat), maps:to_list(Exact), is_map_key({abs}, Flat), maps:find({9, 9}, Flat)].

grid() ->
    Small = cells(0, 0, 4, 8, ?MODULE:id(#{})),
    Big = cells(0, 0, 6, 6, ?MODULE:id(#{})),
    Grown = maps:put({9, 9}, grown, Small),
    Replaced = maps:put({3, 4}, replaced, Big),
    #{{5, 5} := Last} = Big,
    [maps:keys(Small), maps:keys(Big), maps:keys(Grown), map_get({3, 4}, Replaced), maps:get({5, 5}, Big), map_size(Replaced), Last, maps:find({9, 9}, Grown)].

errors() ->
    Map = ?MODULE:id(#{{1, 2} => a}),
    [try_put({1, 2}, b, ?MODULE:id(not_a_map)), try_get({9, 9}, Map), try_get({1, 2}, ?MODULE:id(not_a_map)), try_map_get({9, 9}, Map), try_exact(Map)].

cells(Rows, _, Rows, _, Map) -> Map;
cells(Row, Cols, Rows, Cols, Map) -> cells(Row + 1, 0, Rows, Cols, Map);
cells(Row, Col, Rows, Cols, Map) -> cells(Row, Col + 1, Rows, Cols, maps:put({Row, Col}, Row * 10 + Col, Map)).

try_put(Key, Value, Map) ->
    try maps:put(Key, Value, Map) catch Class:Reason -> {Class, Reason} end.

try_get(Key, Map) ->
    try maps:get(Key, Map) catch Class:Reason -> {Class, Reason} end.

try_map_get(Key, Map) ->
    try map_get(Key, Map) catch Class:Reason -> {Class, Reason} end.

try_exact(Map) ->
    try Map#{{9, 9} := b} catch Class:Reason -> {Class, Reason} end.

id(Term) -> Term.
