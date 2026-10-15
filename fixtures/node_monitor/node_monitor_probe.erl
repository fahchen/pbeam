-module(node_monitor_probe).
-export([main/0, results/0]).

%% Return `ok` when `results/0` gives the values that the pinned ERTS gives
%% on a node that is not alive, else `{mismatch, Results}`.
main() ->
    Expected = [
        false, true, true, {error, badarg},
        0, 1, 2, 3, 0, 4, 0, 0, 1, 0, 0, 0, 1, 0,
        {error, badarg}, {error, badarg}, {error, badarg}, {error, badarg},
        {error, badarg}, {error, badarg}, {error, badarg}, {error, badarg},
        ok, ok, ok,
        [], [], [], [], [nonode@nohost], [nonode@nohost], [], [nonode@nohost],
        {error, badarg}, {error, badarg}, {error, badarg}, {error, badarg},
        [], [{nonode@nohost, #{connection_id => undefined}}],
        [{nonode@nohost, #{}}],
        [{nonode@nohost, #{connection_id => undefined, node_type => this}}],
        [{nonode@nohost, #{}}], [],
        {error, badarg}, {error, badarg}, {error, badarg}, {error, badarg},
        {error, badarg}, {error, badarg}
    ],
    case results() of
        Expected -> ok;
        Results -> {mismatch, Results}
    end.

results() ->
    [
        flag(async_dist, true),
        flag(async_dist, true),
        flag(async_dist, false),
        flag(async_dist, 1),
        flag(monitor_nodes, true),
        flag(monitor_nodes, true),
        flag({monitor_nodes, #{list => true}}, true),
        flag({monitor_nodes, []}, true),
        flag({monitor_nodes, #{}}, true),
        flag(monitor_nodes, false),
        flag(monitor_nodes, false),
        flag({monitor_nodes, #{node_type => visible}}, true),
        flag({monitor_nodes, #{node_type => visible, connection_id => false, nodedown_reason => false}}, true),
        flag({monitor_nodes, #{node_type => hidden}}, true),
        flag({monitor_nodes, #{node_type => all}}, true),
        flag({monitor_nodes, #{node_type => all, connection_id => true, nodedown_reason => true, list => true}}, true),
        flag({monitor_nodes, #{node_type => all}}, false),
        flag({monitor_nodes, #{node_type => all}}, false),
        flag({monitor_nodes, #{node_type => x}}, true),
        flag({monitor_nodes, #{list => false}}, true),
        flag({monitor_nodes, #{a => 1}}, true),
        flag({monitor_nodes, #{1 => true}}, true),
        flag({monitor_nodes, x}, true),
        flag({monitor_nodes, #{}}, x),
        flag(monitor_nodes, 1),
        flag({monitor_nodes, #{}, 1}, true),
        net_kernel:monitor_nodes(true),
        net_kernel:monitor_nodes(true, #{connection_id => true}),
        net_kernel:monitor_nodes(false, [connection_id]),
        nodes(),
        nodes(id(visible)),
        nodes(id(hidden)),
        nodes(id(connected)),
        nodes(id(this)),
        nodes(id(known)),
        nodes(id([])),
        nodes(id([this, known, this])),
        nodes1(foo),
        nodes1([this | visible]),
        nodes1(1),
        nodes1([this, 1]),
        nodes(id(visible), #{connection_id => true}),
        nodes(id(this), #{connection_id => true}),
        nodes(id(known), #{}),
        nodes(id([this, this]), #{node_type => true, connection_id => true}),
        nodes(id(known), #{node_type => false, connection_id => false}),
        nodes(id([]), #{node_type => true}),
        nodes2(this, #{node_type => 1}),
        nodes2(this, #{x => 1}),
        nodes2(this, #{1 => true}),
        nodes2(this, []),
        nodes2(foo, #{}),
        nodes2([], x)
    ].

flag(Flag, Value) ->
    try process_flag(id(Flag), id(Value)) catch Class:Reason -> {Class, Reason} end.

nodes1(NodeTypes) ->
    try nodes(id(NodeTypes)) catch Class:Reason -> {Class, Reason} end.

nodes2(NodeTypes, Options) ->
    try nodes(id(NodeTypes), id(Options)) catch Class:Reason -> {Class, Reason} end.

id(Term) -> Term.
