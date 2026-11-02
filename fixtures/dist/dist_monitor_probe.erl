%% Monitors of a process of one node on a process of another node, in both
%% directions, with the exit reasons normal, kill, and a reason that is not
%% immediate; the removal of a monitor; a pid that is not alive; and the end
%% of the connection (bif.c:758-981, 436-541; dist.c:1496-1600, 2341-2470,
%% 2688-2746, 1021-1117, 305-321; erl_process.c:14101-14147).
-module(dist_monitor_probe).
-export([server/0, client/1]).

%% Print the node name, and make a process for each request of a client.
server() ->
    io:format("~s~n", [node()]),
    register(dmon, self()),
    serve().

serve() ->
    receive
        {victim, From} ->
            _ = spawn(fun() ->
                              From ! {victim, self()},
                              receive
                                  {exit, Why} -> exit(Why);
                                  stop -> ok
                              end
                      end),
            serve();
        {watch, Pid, From} ->
            _ = spawn(fun() ->
                              Ref = monitor(process, Pid),
                              From ! {watching, Pid},
                              receive
                                  {'DOWN', Ref, process, Pid, Why} ->
                                      From ! {watched, Pid, Why}
                              after 20000 -> From ! {watched, Pid, timeout}
                              end
                      end),
            serve();
        halt ->
            erlang:halt()
    end.

%% Run each case with the server node Server and print the results.
client(Server) ->
    true = net_kernel:connect_node(Server),
    Cases = [{Dir, Why} || Dir <- [to, from], Why <- [normal, kill, {boom}]],
    Results = [run(Server, Case) || Case <- Cases],
    io:format("~p~n", [Results ++ [removes(Server), noproc(Server), lost(Server)]]).

run(Server, {to, Why}) ->
    %% A client process monitors a process of the server node.
    V = victim(Server),
    Ref = monitor(process, V),
    V ! {exit, Why},
    {to, Why, down(Ref)};
run(Server, {from, Why}) ->
    %% A process of the server node monitors a client process.
    C = local(false, fun() -> ok end),
    {dmon, Server} ! {watch, C, self()},
    receive {watching, C} -> ok after 10000 -> ok end,
    C ! {exit, Why},
    {from, Why, receive {watched, C, W} -> W after 20000 -> timeout end}.

removes(Server) ->
    %% The removal of a monitor before the exit gives no DOWN message.
    V = victim(Server),
    Ref = monitor(process, V),
    true = demonitor(Ref),
    V ! {exit, boom},
    {demonitor, receive {'DOWN', Ref, _, _, _} -> down after 1000 -> none end}.

noproc(Server) ->
    %% A pid of the server node that is not alive gives noproc.
    V = victim(Server),
    {dmon, Server} ! {watch, V, self()},
    receive {watching, V} -> ok after 10000 -> ok end,
    V ! {exit, boom},
    receive {watched, V, boom} -> ok after 20000 -> timeout end,
    Ref = monitor(process, V),
    {noproc, down(Ref)}.

lost(Server) ->
    %% The end of the connection gives each monitor noconnection, from the
    %% oldest monitor to the newest.
    V1 = victim(Server),
    V2 = victim(Server),
    R1 = monitor(process, V1),
    R2 = monitor(process, V2),
    {dmon, Server} ! halt,
    {lost, order(R1, R2)}.

order(R1, R2) ->
    [down_order(R1, R2), down_order(R1, R2)].

%% Wait for one of the two DOWN messages and say which monitor it ends.
down_order(R1, R2) ->
    receive
        {'DOWN', R1, _, _, Why} -> {oldest, Why};
        {'DOWN', R2, _, _, Why} -> {newest, Why}
    after 20000 -> timeout
    end.

%% Wait for the DOWN message of `Ref` and return its reason, or `timeout`
%% after ten seconds.
down(Ref) ->
    receive {'DOWN', Ref, process, _, Why} -> Why after 10000 -> timeout end.

%% A local process that forwards its messages to the caller.
local(Trap, Body) ->
    Main = self(),
    Pid = spawn(fun() ->
                        process_flag(trap_exit, Trap),
                        Body(),
                        Main ! {ready, self()},
                        forward(Main)
                end),
    receive
        {ready, Pid} -> ok
    end,
    Pid.

forward(Main) ->
    receive
        {exit, Why} -> exit(Why);
        Msg ->
            Main ! {got, self(), Msg},
            forward(Main)
    end.

victim(Server) ->
    {dmon, Server} ! {victim, self()},
    receive {victim, V} -> V end.
