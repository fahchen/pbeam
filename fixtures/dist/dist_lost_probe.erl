%% The links of a connection that ends: a client node links a process that
%% traps exits and a process that does not trap exits to a process of a
%% server node, and the server node halts. The client gives each linked
%% process the exit signal `noconnection` (erts_do_net_exits,
%% dist.c:1021-1117; erl_proc_sig_queue.c:4632-4815).
-module(dist_lost_probe).
-export([server/0, client/1]).

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

%% Link the two client processes to the server process, stop the server
%% node, and print the exit reason that each client process took.
client(Server) ->
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
    receive {ready, Trap} -> ok end,
    receive {ready, Kill} -> ok end,
    {dlost, Server} ! halt,
    Exit = receive {trap, Msg} -> Msg after 20000 -> timeout end,
    Down = receive {'DOWN', Ref, _, _, Why} -> Why after 20000 -> timeout end,
    Result = case Exit of
                 {'EXIT', V, Reason} -> {linked, Reason, Down};
                 _ -> {Exit, Down}
             end,
    io:format("~p~n", [[Result]]).
