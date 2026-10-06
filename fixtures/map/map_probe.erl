-module(map_probe).
-export([main/0, output/1, add/1, same/2, fields/0, pair/1, key/2]).
main() ->
    Options = ?MODULE:add(#{input => cooked}),
    {?MODULE:output(Options), Options#{ofd => stdout}, ?MODULE:output(#{output => cooked}), ?MODULE:output(other),
     ?MODULE:same(Options, #{output => raw, input => cooked})}.
add(M) -> M#{output => raw}.
output(#{output := raw}) -> raw;
output(_) -> other.
same(X, X) -> same;
same(_, _) -> different.
fields() ->
    [?MODULE:pair(#{input => cooked, output => raw}), ?MODULE:pair(#{input => cooked}),
     ?MODULE:pair(#{output => raw}), ?MODULE:pair(raw),
     ?MODULE:key(1, #{1 => one}), ?MODULE:key(2, #{1 => one}), ?MODULE:key(1, raw)].
pair(#{input := _, output := _}) -> both;
pair(_) -> none.
key(K, M) ->
    case M of
        #{K := _} -> found;
        _ -> missing
    end.
