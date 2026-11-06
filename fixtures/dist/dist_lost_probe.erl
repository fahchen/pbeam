%% The links of a connection that ends: a client node links a process that
%% traps exits and a process that does not trap exits to a process of a
%% server node, and the server node halts. The client gives each linked
%% process the exit signal `noconnection` (erts_do_net_exits,
%% dist.c:1021-1117; erl_proc_sig_queue.c:4632-4815).
-module(dist_lost_probe).
-export([server/0, client/1, client_disconnect/1, monitor_local/0, monitor_notalive/0,
         monitor_remote/0, monitor_connected/1]).

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
    io:format("~p~n", [[First, Second, Third, Fourth]]).

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
