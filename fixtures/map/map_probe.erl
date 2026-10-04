-module(map_probe).
-export([main/0, output/1, add/1, same/2]).
main() ->
    Options = ?MODULE:add(#{input => cooked}),
    {?MODULE:output(Options), Options#{ofd => stdout}, ?MODULE:output(#{output => cooked}), ?MODULE:output(other),
     ?MODULE:same(Options, #{output => raw, input => cooked})}.
add(M) -> M#{output => raw}.
output(#{output := raw}) -> raw;
output(_) -> other.
same(X, X) -> same;
same(_, _) -> different.
