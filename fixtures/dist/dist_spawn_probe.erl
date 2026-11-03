%% Spawn processes on a server node with each spawn form, a link, a monitor,
%% a spawn request, the group leader of a remote process, and a node with no
%% connection (bif.c:1027-1075, erl_process.c:12440-12530, dist.c:5850-6260).
-module(dist_spawn_probe).
-export([server/0, client/1, boom/0]).

server() ->
    io:format("~s~n", [node()]),
    register(dspawn, self()),
    receive stop -> erlang:halt() end.

boom() ->
    exit(boom).

client(Server) ->
    true = net_kernel:connect_node(Server),
    process_flag(trap_exit, true),
    P = spawn(Server, erlang, node, []),
    {M, R} = spawn_monitor(Server, erlang, node, []),
    Monitor = receive {'DOWN', R, process, M, Why} -> Why after 5000 -> timeout end,
    _ = spawn_link(Server, dist_spawn_probe, boom, []),
    Link = receive {'EXIT', _, Why2} -> Why2 after 5000 -> timeout end,
    Req = spawn_request(Server, dist_spawn_probe, boom, [], [monitor]),
    Request = receive {spawn_reply, Req, ok, _} -> ok; {spawn_reply, Req, error, E} -> E
              after 5000 -> timeout
              end,
    RequestDown = receive {'DOWN', Req, process, _, Why3} -> Why3 after 5000 -> timeout end,
    Host = lists:nth(2, string:split(atom_to_list(Server), "@")),
    Missing = spawn_request(list_to_atom("none@" ++ Host), erlang, node, [], [monitor]),
    NoConnection = receive {spawn_reply, Missing, error, Reason} -> Reason
                   after 5000 -> timeout
                   end,
    Lost = stopped(Server),
    io:format("~0p~n", [[{spawn, node(P) =:= Server}, {monitor, Monitor},
                         {link, Link}, {request, Request}, {request_down, RequestDown},
                         {noconnection, NoConnection}, {lost, Lost}]]),
    ok.

stopped(Server) ->
    Watched = monitor(process, {dspawn, Server}),
    {dspawn, Server} ! stop,
    receive {'DOWN', Watched, _, _, Why} -> Why after 10000 -> timeout end.
