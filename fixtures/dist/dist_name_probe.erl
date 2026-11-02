%% The monitor of a registered name of a process of another node, which the
%% MONITOR_P packet asks for: the node looks the name up when it gets the
%% packet, gives the monitor to the process that the name has then, and
%% answers MONITOR_P_EXIT with noproc for a name that no process has
%% (dist.c:2364-2379, 2398-2410, 2431-2459; erl_process.c:13844-13847). The
%% local erlang:monitor/2,3 of {Name, Node} stays out of scope (#1494), so
%% the client always runs on ERTS here and only the server changes.
-module(dist_name_probe).
-export([server/0, client/1]).

%% Print the node name, register `dmname` on a process that waits for an
%% exit reason, and answer each request of a client.
server() ->
    io:format("~s~n", [node()]),
    register(dmon, self()),
    Server = self(),
    _ = spawn(fun() ->
                      register(dmname, self()),
                      Server ! {registered, self()},
                      receive
                          {exit, Why} -> exit(Why)
                      end
              end),
    receive
        {registered, _} -> ok
    end,
    serve().

serve() ->
    receive
        {victim, From} -> From ! {victim, whereis(dmname)}, serve();
        stop -> erlang:halt()
    end.

%% Monitor the name of the server node, keep one monitor to the end, remove
%% one before the exit, then remove a monitor that ended, and ask for a name
%% that no process has. The client stops the server node after its result, so
%% the test records the coverage of the runs of the server: a node that the
%% test kills writes no profile.
client(Server) ->
    true = net_kernel:connect_node(Server),
    Victim = victim(Server),
    Removed = monitor(process, {dmname, Server}),
    %% The monitor is there, so the removal flushes nothing and gives true
    %% (bif.c:436-541).
    true = demonitor(Removed, [flush]),
    Kept = monitor(process, {dmname, Server}),
    Victim ! {exit, boom},
    Down = down(Kept),
    %% The removal of a monitor whose target exited still sends its
    %% DEMONITOR_P (bif.c:525-570), and the name then names the target of no
    %% monitor of the connection, so the node removes nothing
    %% (dist.c:2441-2457).
    Late = demonitor(Kept, [flush]),
    Kept2 = receive {'DOWN', Removed, _, _, _} -> down after 1000 -> none end,
    Missing = monitor(process, {'no such name', Server}),
    Noproc = down(Missing),
    io:format("~p~n", [[{down, Down}, {removed, Kept2}, {late, Late}, {noproc, Noproc}]]),
    stopped(Server).

%% Stop the node `Server` and wait until the connection to it ends, as the
%% end of the node closes it. The run of a PBeam node writes its own
%% profile when it ends on its own; a run that the test kills writes none.
stopped(Server) ->
    Watched = monitor(process, {dmon, Server}),
    {dmon, Server} ! stop,
    receive {'DOWN', Watched, _, _, _} -> ok after 10000 -> timeout end,
    ok.

victim(Server) ->
    {dmon, Server} ! {victim, self()},
    receive {victim, Victim} -> Victim after 10000 -> undefined end.

%% Wait for the DOWN message of `Ref` and return its reason, or `timeout`
%% after ten seconds.
down(Ref) ->
    receive {'DOWN', Ref, process, _, Why} -> Why after 10000 -> timeout end.
