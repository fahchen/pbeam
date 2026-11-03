%% net_adm:ping/1 and gen_server:call/2 to a registered name of another node
%% (net_adm.erl:101-110, gen.erl, gen_server.erl). `gen:do_call/4` monitors
%% the target with the `demonitor` alias option and waits for the reply
%% through the alias.
-module(dist_call_probe).
-behaviour(gen_server).
-export([server/0, client/1, unconnected/1]).
-export([init/1, handle_call/3, handle_cast/2, handle_info/2, terminate/2,
         code_change/3]).

%% Print the node name, start the `dcall` server, and answer the stop of a
%% client.
server() ->
    io:format("~s~n", [node()]),
    {ok, _} = gen_server:start_link({local, dcall}, ?MODULE, [], []),
    register(dsrv, self()),
    receive stop -> erlang:halt() end.

init([]) ->
    {ok, ok}.

handle_call({echo, Value}, _From, State) ->
    {reply, Value, State};
handle_call(_Request, _From, State) ->
    {reply, ok, State}.

handle_cast(_Request, State) ->
    {noreply, State}.

handle_info(_Message, State) ->
    {noreply, State}.

terminate(_Reason, _State) ->
    ok.

code_change(_OldVersion, State, _Extra) ->
    {ok, State}.

%% Connect, then ping a live node and a name that is not a node, and call the
%% `dcall` server of the node and a name that no process has. The client stops
%% the server node after its result, so the test records the coverage of the
%% runs of the server: a node that the test kills writes no profile.
client(Server) ->
    true = net_kernel:connect_node(Server),
    Ping = net_adm:ping(Server),
    Pang = net_adm:ping('none@nohost'),
    Reply = gen_server:call({dcall, Server}, {echo, 42}),
    Noproc = try gen_server:call({absent, Server}, ping)
             catch exit:{noproc, {gen_server, call, _}} -> noproc;
                   exit:Reason -> Reason
             end,
    Lost = stopped(Server),
    io:format("~0p~n", [[{ping, Ping}, {pang, Pang}, {reply, Reply},
                         {noproc, Noproc}, {lost, Lost}]]),
    ok.

%% Ping the live node `Server` with no connection before the call.
%% `net_adm:ping/1` makes a `gen:call` to `{net_kernel, Server}`, so a node
%% connects on that call in ERTS, and the monitor of the name of a node with
%% no connection ends with `noconnection` in PBeam (#1412).
unconnected(Server) ->
    io:format("~p~n", [[{unconnected, net_adm:ping(Server)}]]),
    ok.

%% Monitor the name of the server process, stop the node, and return the
%% reason of the DOWN message, which the end of the connection gives.
stopped(Server) ->
    Watched = monitor(process, {dsrv, Server}),
    {dsrv, Server} ! stop,
    receive {'DOWN', Watched, _, _, Why} -> Why after 10000 -> timeout end.
