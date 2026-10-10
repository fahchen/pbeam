-module(tuple_key_probe).
-export([main/0, id/1]).

main() ->
    Flat = ?MODULE:id(#{{1, 2} => a, {0, 5} => b, {1} => c, {value} => d, {abs} => e, value => f, abs => g, 3 => h, [1] => i, {} => j, {[1], {2}} => k}),
    #{{1, 2} := A, {value} := D} = Flat,
    Key = ?MODULE:id({2, 1}),
    Assoc = Flat#{Key => l, {1, 2} => m},
    Exact = (?MODULE:id(Assoc))#{{0, 5} := n},
    [maps:keys(Flat), A, D, map_get({1}, Flat), maps:get({}, Flat), maps:to_list(Exact), is_map_key({abs}, Flat), maps:find({9, 9}, Flat)].

id(Term) -> Term.
