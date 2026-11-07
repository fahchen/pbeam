%% A distribution owner remains alive after its registered name is removed.
-module(dist_coverage_probe).
-export([missing_kernel/0]).

missing_kernel() ->
    Parent = self(),
    Kernel = spawn(fun() ->
                           true = register(net_kernel, self()),
                           Parent ! ready,
                           receive stop -> ok end
                   end),
    receive ready -> ok end,
    true = erlang:setnode(coverage_local@nohost, 4),
    true = unregister(net_kernel),
    Node = coverage_peer@nohost,
    Handle = erts_internal:new_connection(Node),
    true = erts_internal:abort_pending_connection(Node, Handle),
    A = erlang:monitor_node(Node, true),
    B = receive {nodedown, Node} -> nodedown after 1000 -> timeout end,
    Ref = spawn_request(Node, erlang, node, [], []),
    C = receive {spawn_reply, Ref, error, Why} -> Why after 1000 -> timeout end,
    Silent = spawn_request(Node, erlang, node, [], [{reply, no}]),
    D = receive {spawn_reply, Silent, _, _} -> unexpected after 100 -> none end,
    Kernel ! stop,
    io:format("~w~n", [{A, B, C, D}]).
