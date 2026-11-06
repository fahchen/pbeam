%% The links of a connection that ends: a client node links a process that
%% traps exits and a process that does not trap exits to a process of a
%% server node, and the server node halts. The client gives each linked
%% process the exit signal `noconnection` (erts_do_net_exits,
%% dist.c:1021-1117; erl_proc_sig_queue.c:4632-4815).
-module(dist_lost_probe).
-export([server/0, client/1, client_disconnect/1, monitor_local/0, monitor_notalive/0,
         monitor_remote/0, monitor_connected/1, monitor_events/1, monitor_forms/1,
         monitor_hidden/1, monitor_local_events/0, ordering_server/0, ordering_client/1,
         local_order/1, local_race/0, local_multi/0, controller_reasons/0, no_network/0,
         self_signal/1, monitor_disable_signal/1, nodes_forms/2, nodes_order/0, nodes_expiry/0,
         disconnect_cases/1, disconnect_notalive/0, monitor_badopt/0]).

monitor_badopt() ->
    Call = fun(Node, Enabled, Options) ->
                   try erlang:monitor_node(Node, Enabled, Options)
                   catch error:badarg:Stack ->
                             [{erlang, monitor_node, _, Info} | _] = Stack,
                             proplists:get_value(error_info, Info)
                   end
           end,
    io:format("~w~n", [[Call(node(), true, bad), Call(node(), true, [bad]),
                         Call(node(), true, [allow_passive_connect | bad]),
                         Call(1, invalid, [bad]), Call(1, invalid, []),
                         Call(node(), invalid, [])]]).

disconnect_cases(Server) ->
    false = erlang:disconnect_node(node()),
    false = erlang:disconnect_node(42),
    false = erlang:disconnect_node('nmemissing@localhost'),
    true = net_kernel:connect_node(Server),
    true = erlang:monitor_node(Server, true),
    Result = erlang:disconnect_node(Server),
    Down = receive {nodedown, Server} -> nodedown after 5000 -> timeout end,
    Again = erlang:disconnect_node(Server),
    Connected = lists:member(Server, erlang:nodes(connected)),
    io:format("~w~n", [[Result, Down, Again, Connected]]).

disconnect_notalive() ->
    io:format("~w~n", [[erlang:disconnect_node(node()), erlang:disconnect_node(42),
                        erlang:disconnect_node('nmemissing@localhost')]]).

nodes_forms(Visible, Hidden) ->
    true = net_kernel:connect_node(Visible),
    true = net_kernel:connect_node(Hidden),
    Local = node(),
    Names = fun(L) -> [case N of Visible -> visible_peer; Hidden -> hidden_peer; Local -> local; 'nonode@nohost' -> nonode end || N <- L, N =:= Visible orelse N =:= Hidden orelse N =:= Local orelse N =:= 'nonode@nohost'] end,
    Info = fun(L) -> [{hd(Names([N])), maps:get(node_type, I), case maps:get(connection_id, I) of undefined -> undefined; C when is_integer(C) -> integer end} || {N, I} <- L, N =:= Visible orelse N =:= Hidden orelse N =:= Local orelse N =:= 'nonode@nohost'] end,
    Types = [visible, connected, hidden, this, known, [], [this, connected], [known, hidden]],
    Result = [{T, Names(erlang:nodes(T)), Info(erlang:nodes(T, #{node_type => true, connection_id => true}))} || T <- Types],
    io:format("~w~n", [Result]).

nodes_order() ->
    A = 'nmea@host', B = 'nmeb@host', C = 'nmec@host',
    noconnect = erlang:send({reg, A}, msg, [noconnect]),
    Handle = erts_internal:new_connection(B),
    true = erts_internal:abort_pending_connection(B, Handle),
    noconnect = erlang:send({reg, B}, msg, [noconnect]),
    noconnect = erlang:send({reg, C}, msg, [noconnect]),
    Filter = fun() -> [N || N <- erlang:nodes(known), N =:= A orelse N =:= B orelse N =:= C] end,
    Before = Filter(),
    HA = erts_internal:new_connection(A),
    true = erts_internal:abort_pending_connection(A, HA),
    io:format("~w~n", [[Before, Filter()]]).

nodes_expiry() ->
    A = 'nmeexpire@host', B = 'nmekeep@host',
    noconnect = erlang:send({reg, A}, msg, [noconnect]),
    Handle = erts_internal:new_connection(B),
    true = erts_internal:abort_pending_connection(B, Handle),
    put(handle, Handle),
    timer:sleep(62000),
    io:format("~w~n", [[N || N <- erlang:nodes(known), N =:= A orelse N =:= B]]).

monitor_disable_signal(Disabled) ->
    logger:set_primary_config(level, none),
    Node = 'nmetarget@localhost',
    Flags = element(3, erts_internal:get_dflags()),
    _ = erts_internal:new_connection(Node),
    {ok, Port} = gen_tcp:listen(0, []),
    {ok, _} = erts_internal:create_dist_channel(Node, Port, {Flags, 1}),
    true = erlang:monitor_node(Node, true),
    true = erlang:monitor_node(Node, true),
    true = erlang:port_close(Port),
    [true = erlang:monitor_node(Node, false) || _ <- lists:seq(1, Disabled)],
    Count = fun F(N) -> receive {nodedown, Node} -> F(N + 1) after 100 -> N end end,
    io:format("~p~n", [Count(0)]).

self_signal(Mode) ->
    true = register(net_kernel, self()),
    0 = erlang:process_flag(monitor_nodes, true),
    true = erlang:setnode('nmeself@localhost', 4),
    case Mode of
        disable -> 1 = erlang:process_flag(monitor_nodes, false);
        increase -> 1 = erlang:process_flag(monitor_nodes, true);
        recreate -> 1 = erlang:process_flag(monitor_nodes, false), 0 = erlang:process_flag(monitor_nodes, true)
    end,
    Count = fun F(N) -> receive {nodeup, 'nmeself@localhost'} -> F(N + 1) after 0 -> N end end,
    io:format("~p~n", [Count(0)]).

no_network() ->
    ok = net_kernel:monitor_nodes(true, #{nodedown_reason => true}),
    Parent = self(),
    Pid = spawn(fun() -> register(net_kernel, self()), Parent ! ready, receive stop -> exit(no_network) end end),
    receive ready -> ok end,
    true = erlang:setnode('nmenetwork@localhost', 4),
    receive {nodeup, 'nmenetwork@localhost', _} -> ok end,
    Pid ! stop,
    Down = receive {nodedown, 'nmenetwork@localhost', Info} -> maps:get(nodedown_reason, Info) after 5000 -> timeout end,
    io:format("~p~n", [Down]).

controller_reasons() ->
    logger:set_primary_config(level, none),
    ok = net_kernel:monitor_nodes(true, #{node_type => hidden, nodedown_reason => true}),
    Flags = element(3, erts_internal:get_dflags()),
    Run = fun({Name, Reason}) ->
                  _ = erts_internal:new_connection(Name),
                  Pid = spawn(fun() -> receive stop -> case Reason of normal -> ok; _ -> exit(Reason) end end end),
                  {message, Ref} = erts_internal:create_dist_channel(Name, Pid, {Flags, 1}),
                  receive {Ref, _} -> ok end,
                  receive {nodeup, Name, _} -> ok end,
                  Pid ! stop,
                  receive {nodedown, Name, Info} -> maps:get(nodedown_reason, Info) after 5000 -> timeout end
          end,
    io:format("~p~n", [[Run(C) || C <- [{'nmenormal@localhost', normal}, {'nmeshutdown@localhost', {shutdown, custom}}, {'nmeerror@localhost', custom}]]]).

local_multi() ->
    logger:set_primary_config(level, none),
    ok = net_kernel:monitor_nodes(true, #{node_type => all}),
    {ok, _} = net_kernel:start(nmemulti, #{name_domain => shortnames}),
    Local = node(),
    receive {nodeup, Local, _} -> ok after 5000 -> error(local_up_timeout) end,
    Flags = element(3, erts_internal:get_dflags()),
    Start = fun(Node) ->
                    _ = erts_internal:new_connection(Node),
                    Pid = spawn(fun() -> receive never -> ok end end),
                    {message, Ref} = erts_internal:create_dist_channel(Node, Pid, {Flags, 1}),
                    receive {Ref, _} -> ok after 5000 -> error(channel_timeout) end,
                    receive {nodeup, Node, _} -> ok after 5000 -> error(remote_up_timeout) end,
                    Pid
            end,
    A = 'nmea@localhost', B = 'nmeb@localhost',
    P1 = Start(A), P2 = Start(B),
    spawn(fun() -> exit(P1, kill) end),
    ok = net_kernel:stop(),
    First = receive {nodedown, X, _} -> X after 5000 -> timeout end,
    Second = receive {nodedown, Y, _} -> Y after 5000 -> timeout end,
    Last = receive {nodedown, Z, _} -> Z after 5000 -> timeout end,
    io:format("~p~n", [[lists:sort([First, Second]) =:= lists:sort([A, B]), Last =:= Local,
                        is_process_alive(P1), is_process_alive(P2)]]).

local_race() ->
    ok = net_kernel:monitor_nodes(true, #{node_type => all}),
    Parent = self(),
    Check = fun(_) ->
                    Pid = spawn(fun() -> register(net_kernel, self()), Parent ! {ready, self()}, receive go -> ok end end),
                    Ref = monitor(process, Pid),
                    receive {ready, Pid} -> ok end,
                    spawn(fun() -> Pid ! go end),
                    Result = try erlang:setnode('nmerace@localhost', 4) catch error:badarg -> badarg end,
                    receive {'DOWN', Ref, process, Pid, _} -> ok end,
                    case Result of
                        true -> Up = receive {nodeup, 'nmerace@localhost', _} -> ok; {nodedown, 'nmerace@localhost', _} -> down_first after 5000 -> up_timeout end,
                                Down = receive {nodedown, 'nmerace@localhost', _} -> ok after 5000 -> down_timeout end,
                                {Up, Down};
                        badarg -> {ok, ok}
                    end
            end,
    io:format("~p~n", [lists:all(fun(V) -> V =:= {ok, ok} end, [Check(I) || I <- lists:seq(1, 12)])]).

local_order(Server) ->
    ok = net_kernel:monitor_nodes(true, #{node_type => all}),
    Check = fun(I) ->
                    Name = list_to_atom("nmelocal" ++ integer_to_list(I)),
                    {ok, _} = net_kernel:start(Name, #{name_domain => shortnames}),
                    Local = node(),
                    Up = receive {nodeup, Local, _} -> up; {nodedown, Local, _} -> down_first after 5000 -> timeout end,
                    true = net_kernel:connect_node(Server),
                    receive {nodeup, Server, _} -> ok after 5000 -> error(nodeup_timeout) end,
                    ok = net_kernel:stop(),
                    First = receive {nodedown, X, _} -> X after 5000 -> timeout end,
                    Second = receive {nodedown, X2, _} -> X2 after 5000 -> timeout end,
                    {Up, First =:= Server, Second =:= Local}
            end,
    io:format("~p~n", [[Check(I) || I <- lists:seq(1, 3)]]).

ordering_server() ->
    ok = net_kernel:monitor_nodes(true, #{node_type => all}),
    io:format("~s~n", [node()]),
    ordering_loop().

ordering_loop() ->
    receive {nodeup, Node, _} -> erlang:disconnect_node(Node), ordering_loop();
            _ -> ordering_loop()
    end.

ordering_client(Server) ->
    ok = net_kernel:monitor_nodes(true, #{node_type => all}),
    Check = fun(_) ->
                    _ = net_kernel:connect_node(Server),
                    receive
                        {nodeup, Server, _} ->
                            receive {nodedown, Server, _} -> ok after 5000 -> down_timeout end;
                        {nodedown, Server, _} -> down_first
                    after 5000 -> up_timeout
                    end
            end,
    io:format("~p~n", [[Check(N) || N <- lists:seq(1, 12)]]).

monitor_local_events() ->
    ok = net_kernel:monitor_nodes(true, #{node_type => all, connection_id => true, nodedown_reason => true}),
    {ok, _} = net_kernel:start(nmelocal, #{name_domain => shortnames}),
    Node = node(),
    Up = receive {nodeup, Node, U} -> U after 5000 -> timeout end,
    ok = net_kernel:stop(),
    Down = receive {nodedown, Node, D} -> D after 5000 -> timeout end,
    io:format("~p~n", [[Up, Down]]).

monitor_hidden(Server) ->
    ok = net_kernel:monitor_nodes(true),
    ok = net_kernel:monitor_nodes(true, #{node_type => visible}),
    ok = net_kernel:monitor_nodes(true, #{node_type => hidden}),
    true = net_kernel:connect_node(Server),
    Up = receive {nodeup, Server, U} -> U after 5000 -> timeout end,
    Extra = receive {nodeup, Server} -> plain; {nodeup, Server, _} -> other after 100 -> none end,
    true = erlang:disconnect_node(Server),
    Down = receive {nodedown, Server, D} -> D after 5000 -> timeout end,
    io:format("~p~n", [[Up, Extra, Down]]).

monitor_forms(Server) ->
    ok = net_kernel:monitor_nodes(true),
    ok = net_kernel:monitor_nodes(true),
    ok = net_kernel:monitor_nodes(true, [connection_id, {node_type, all}, nodedown_reason]),
    ok = net_kernel:monitor_nodes(true, #{connection_id => true, node_type => all, nodedown_reason => true}),
    true = net_kernel:connect_node(Server),
    A = receive {nodeup, Server} -> up after 5000 -> timeout end,
    B = receive {nodeup, Server} -> up after 5000 -> timeout end,
    L = receive {nodeup, Server, X} when is_list(X) -> X after 5000 -> timeout end,
    M = receive {nodeup, Server, X2} when is_map(X2) -> X2 after 5000 -> timeout end,
    Id = maps:get(connection_id, M),
    ok = net_kernel:monitor_nodes(false),
    true = erlang:disconnect_node(Server),
    D = receive {nodedown, Server} -> down after 100 -> none end,
    DL = receive {nodedown, Server, Y} when is_list(Y) -> Y after 5000 -> timeout end,
    DM = receive {nodedown, Server, Y2} when is_map(Y2) -> Y2 after 5000 -> timeout end,
    io:format("~p~n", [[A,B,D, is_integer(Id), proplists:get_value(connection_id,L)=:=Id,
                        proplists:get_value(connection_id,DL)=:=Id,
                        maps:get(connection_id,DM)=:=Id,
                        proplists:get_value(node_type,L), maps:get(node_type,M),
                        proplists:get_value(nodedown_reason,DL), maps:get(nodedown_reason,DM)]]).

monitor_events(Server) ->
    ok = net_kernel:monitor_nodes(true, #{node_type => all, nodedown_reason => true}),
    true = net_kernel:connect_node(Server),
    Up = receive {nodeup, Server, U} -> U after 5000 -> timeout end,
    true = erlang:disconnect_node(Server),
    Down = receive {nodedown, Server, D} -> D after 5000 -> timeout end,
    io:format("~p~n", [[Up, Down]]).

monitor_connected(Server) ->
    true = erlang:monitor_node(foo, false),
    true = erlang:monitor_node(Server, false),
    true = net_kernel:connect_node(Server),
    true = erlang:monitor_node(Server, true),
    true = erlang:monitor_node(Server, true, []),
    true = erlang:monitor_node(Server, false),
    true = erlang:disconnect_node(Server),
    First = receive {nodedown, Server} -> nodedown after 5000 -> timeout end,
    Second = receive {nodedown, Server} -> nodedown after 100 -> none end,
    true = erlang:monitor_node(Server, false),
    true = net_kernel:connect_node(Server),
    true = erlang:monitor_node(Server, true),
    true = erlang:monitor_node(Server, true),
    {Child, Ref} = spawn_monitor(fun() -> erlang:monitor_node(Server, true) end),
    receive {'DOWN', Ref, process, Child, normal} -> ok end,
    true = erlang:disconnect_node(Server),
    Third = receive {nodedown, Server} -> nodedown after 5000 -> timeout end,
    Fourth = receive {nodedown, Server} -> nodedown after 5000 -> timeout end,
    true = net_kernel:connect_node(Server),
    true = erlang:monitor_node(Server, true),
    true = erlang:monitor_node(Server, false),
    true = erlang:disconnect_node(Server),
    Fifth = receive {nodedown, Server} -> nodedown after 100 -> none end,
    io:format("~p~n", [[First, Second, Third, Fourth, Fifth]]).

monitor_remote() -> erlang:monitor_node(foo, true).

monitor_notalive() ->
    Result = try erlang:monitor_node(foo, true) catch error:R -> {error, R} end,
    io:format("~p~n", [Result]).

monitor_local() ->
    Call = fun(N, B, O) ->
                   try erlang:monitor_node(N, B, O) catch error:R -> {error, R} end
           end,
    io:format("~p~n", [[erlang:monitor_node(node(), true),
                        Call(node(), false, [allow_passive_connect]),
                        Call(node(), invalid, []), Call(1, true, []),
                        Call(node(), true, [invalid]), Call(node(), true, invalid)]]).

%% Print the node name, and make a process for each request of a client.
server() ->
    io:format("~s~n", [node()]),
    register(dlost, self()),
    serve().

serve() ->
    receive
        {victim, From} ->
            _ = spawn(fun() ->
                              From ! {victim, self()},
                              receive stop -> ok end
                      end),
            serve();
        halt ->
            erlang:halt()
    end.

%% Link two client processes and monitor the server process. Stop the server
%% node and print the reasons of the link exit and monitor down signals.
client(Server) ->
    client(Server, halt).

client_disconnect(Server) ->
    client(Server, disconnect).

client(Server, Action) ->
    true = net_kernel:connect_node(Server),
    {dlost, Server} ! {victim, self()},
    V = receive {victim, Pid} -> Pid end,
    Main = self(),
    Trap = spawn(fun() ->
                         process_flag(trap_exit, true),
                         true = link(V),
                         Main ! {ready, self()},
                         receive Msg -> Main ! {trap, Msg} end
                 end),
    Kill = spawn(fun() ->
                         true = link(V),
                         Main ! {ready, self()},
                         receive stop -> ok end
                 end),
    Ref = monitor(process, Kill),
    RemoteRef = monitor(process, V),
    receive {ready, Trap} -> ok end,
    receive {ready, Kill} -> ok end,
    case Action of
        halt -> {dlost, Server} ! halt;
        disconnect -> true = erlang:disconnect_node(Server)
    end,
    Exit = receive {trap, Msg} -> Msg after 20000 -> timeout end,
    Down = receive {'DOWN', Ref, _, _, Why} -> Why after 20000 -> timeout end,
    RemoteDown = receive {'DOWN', RemoteRef, process, V, RemoteWhy} -> RemoteWhy
                 after 20000 -> timeout end,
    Result = case Exit of
                 {'EXIT', V, Reason} -> {linked, Reason, Down, RemoteDown};
                 _ -> {Exit, Down, RemoteDown}
             end,
    io:format("~p~n", [[Result]]).
