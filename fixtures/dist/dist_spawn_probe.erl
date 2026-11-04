%% Spawn processes on a server node with each spawn form, a link, a monitor,
%% a spawn request, the reply options, the group leader of a remote process,
%% and a node with no connection (bif.c:1027-1075,
%% erl_process.c:12440-12530, dist.c:5850-6260).
-module(dist_spawn_probe).
-export([server/0, client/1, leader/1, options/1, report/1, boom/0]).

server() ->
    io:format("~s~n", [node()]),
    register(dspawn, self()),
    receive stop -> erlang:halt() end.

boom() ->
    exit(boom).

%% Report the group leader of the new process, and write a line on it.
report(Client) ->
    io:format("gl~n"),
    Client ! {leader, group_leader()},
    ok.

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
    %% An option that the receiver rejects answers `badopt` with no process
    %% (`dist.c:2931-2980`).
    Unknown = spawn_request(Server, erlang, node, [], [monitor, foo]),
    UnknownReply = receive {spawn_reply, Unknown, error, Why4} -> Why4
                   after 5000 -> timeout
                   end,
    UnknownSpawn = try spawn_opt(Server, erlang, node, [], [foo]) of
                       _ -> value
                   catch
                       error:badarg -> badarg
                   end,
    %% The `{reply, ...}` options of a spawn request keep or suppress the
    %% message of a successful spawn and of a failed spawn
    %% (`dist.c:5901-5924`, `erl_proc_sig_queue.c:4877-4900`, `5845`).
    NoSuccess = spawn_request(Server, erlang, node, [], [monitor, {reply, no}]),
    NoSuccessMessage = received(NoSuccess),
    NoSuccessDown = down(NoSuccess),
    ErrorOnly = spawn_request(Server, erlang, node, [], [monitor, {reply, error_only}]),
    ErrorOnlyMessage = received(ErrorOnly),
    ErrorOnlyDown = down(ErrorOnly),
    SuccessOnly = spawn_request(Server, erlang, node, [],
                                [{reply, success_only}, {monitor, [foo]}]),
    SuccessOnlyMessage = received(SuccessOnly),
    NoError = spawn_request(Server, erlang, node, [], [{reply, no}, {monitor, [foo]}]),
    NoErrorMessage = received(NoError),
    Host = lists:nth(2, string:split(atom_to_list(Server), "@")),
    Missing = spawn_request(list_to_atom("none@" ++ Host), erlang, node, [], [monitor]),
    NoConnection = receive {spawn_reply, Missing, error, Reason} -> Reason
                   after 5000 -> timeout
                   end,
    Lost = stopped(Server),
    %% A bad option of the request itself is `badopt` for a spawn request
    %% and `badarg` for a spawn, with no connection needed
    %% (`dist.c:5944-5968`, `6190-6211`).
    BadMonitor = spawn_request(Server, erlang, node, [], [{monitor, [foo]}]),
    BadMonitorReply = receive {spawn_reply, BadMonitor, error, Why5} -> Why5
                      after 5000 -> timeout
                      end,
    BadLink = spawn_request(Server, erlang, node, [], [{link, [foo]}]),
    BadLinkReply = receive {spawn_reply, BadLink, error, Why6} -> Why6
                   after 5000 -> timeout
                   end,
    BadOption = try spawn_opt(Server, erlang, node, [], [{monitor, [foo]}]) of
                    _ -> value
                catch
                    error:badarg -> badarg
                end,
    io:format("~0p~n", [[{spawn, node(P) =:= Server}, {monitor, Monitor},
                         {link, Link}, {request, Request}, {request_down, RequestDown},
                         {noconnection, NoConnection}, {lost, Lost},
                         {badopt, {BadMonitorReply, BadLinkReply, BadOption}},
                         {unknown, {UnknownReply, UnknownSpawn}},
                         {reply, [{no, {NoSuccessMessage, NoSuccessDown}},
                                  {error_only, {ErrorOnlyMessage, ErrorOnlyDown}},
                                  {success_only, SuccessOnlyMessage},
                                  {no_error, NoErrorMessage}]}]]),
    ok.

%% The new process gets the group leader of the caller, so a PBeam node that
%% keeps no group leader of another node gives the process the group leader
%% of its root process (row "Distribution").
leader(Server) ->
    true = net_kernel:connect_node(Server),
    GroupLeader = group_leader(),
    _ = spawn_opt(Server, dist_spawn_probe, report, [self()], []),
    Leader = receive {leader, L} -> L after 5000 -> timeout end,
    io:format("leader: ~p~n", [leader_result(Leader, GroupLeader)]),
    ok.

leader_result(timeout, _GroupLeader) ->
    timeout;
leader_result(Leader, GroupLeader) ->
    [{Leader =:= GroupLeader, node(Leader) =:= node(GroupLeader)}].

%% The argument checks of `erts_internal:dist_spawn_request/4` return the
%% atom `badarg` (`dist.c:5860-5975`), and its option list gives the reply
%% policy, the tag, and the link and monitor options (`dist.c:5880-5975`).
options(Server) ->
    true = net_kernel:connect_node(Server),
    BadNode = erts_internal:dist_spawn_request(make_ref(), {erlang, node, []}, [],
                                               spawn_request),
    Local = erts_internal:dist_spawn_request(node(), {erlang, node, []}, [],
                                             spawn_request),
    BadOptions = erts_internal:dist_spawn_request(Server, {erlang, node, []},
                                                  [link | foo], spawn_request),
    BadMfa = erts_internal:dist_spawn_request(Server, foo, [], spawn_request),
    BadMfaElement = erts_internal:dist_spawn_request(Server, {1, node, []}, [],
                                                     spawn_request),
    BadArguments = erts_internal:dist_spawn_request(Server, {erlang, node, foo},
                                                    [], spawn_request),
    ReplyTagSpawn = try spawn_opt(Server, erlang, node, [], [{reply_tag, mine}]) of
                        _ -> value
                    catch
                        error:badarg -> badarg
                    end,
    ReplySpawn = try spawn_opt(Server, erlang, node, [], [{reply, no}]) of
                     _ -> value
                 catch
                     error:badarg -> badarg
                 end,
    LinkSpawn = try spawn_opt(Server, erlang, node, [], [{link, [foo]}]) of
                    _ -> value
                catch
                    error:badarg -> badarg
                end,
    Tagged = spawn_request(Server, erlang, node, [], [{reply_tag, mine}]),
    BadLinkNotList = received(spawn_request(Server, erlang, node, [],
                                            [{link, foo}])),
    BadMonitorNotList = received(spawn_request(Server, erlang, node, [],
                                               [{monitor, foo}])),
    BadReply = received(spawn_request(Server, erlang, node, [], [{reply, foo}])),
    BadReplyValue = received(spawn_request(Server, erlang, node, [], [{reply, 1}])),
    BadMonitorAlias = received(spawn_request(Server, erlang, node, [],
                                             [{monitor, [{alias, foo}]}])),
    BadMonitorTag = received(spawn_request(Server, erlang, node, [],
                                           [{monitor, [{tag, mine, other}]}])),
    BadTuple = received(spawn_request(Server, erlang, node, [], [{foo, bar}])),
    LinkEmpty = received(spawn_request(Server, erlang, node, [], [{link, []}])),
    MonitorEmpty = received(spawn_request(Server, erlang, node, [],
                                          [{monitor, []}])),
    MonitorTag = received(spawn_request(Server, erlang, node, [],
                                        [{monitor, [{tag, mine}]}])),
    ReplyYes = received(spawn_request(Server, erlang, node, [], [{reply, yes}])),
    Host = lists:nth(2, string:split(atom_to_list(Server), "@")),
    Missing = list_to_atom("none@" ++ Host),
    %% A suppressed error message of a request that never left, and of a
    %% request that the receiver rejects (`dist.c:6111-6211`,
    %% `erl_proc_sig_queue.c:4877-4900`).
    NoConnection = received(spawn_request(Missing, erlang, node, [],
                                          [{reply, no}, monitor])),
    NoReceiver = received(spawn_request(Server, erlang, node, [],
                                        [{reply, no}, foo])),
    io:format("options: ~0p~n",
              [[{bad_node, BadNode}, {local, Local},
                {bad_options, BadOptions}, {bad_mfa, BadMfa},
                {bad_mfa_element, BadMfaElement}, {bad_arguments, BadArguments},
                {reply_tag_spawn, ReplyTagSpawn}, {reply_spawn, ReplySpawn},
                {link_spawn, LinkSpawn}, {reply_tag_request, is_reference(Tagged)},
                {bad_link_not_list, BadLinkNotList},
                {bad_monitor_not_list, BadMonitorNotList},
                {bad_reply, BadReply}, {bad_reply_value, BadReplyValue},
                {bad_monitor_alias, BadMonitorAlias},
                {bad_monitor_tag, BadMonitorTag}, {bad_tuple, BadTuple},
                {link_empty, LinkEmpty}, {monitor_empty, MonitorEmpty},
                {monitor_tag, MonitorTag}, {reply_yes, ReplyYes},
                {no_connection, NoConnection}, {no_receiver, NoReceiver}]]),
    ok.

%% Return whether the request gave a message within half a second.
received(Request) ->
    receive {spawn_reply, Request, _, _} -> message after 500 -> none end.

%% Return the reason of the `DOWN` message of the request.
down(Request) ->
    receive {'DOWN', Request, _, _, Why} -> Why after 5000 -> timeout end.

stopped(Server) ->
    Watched = monitor(process, {dspawn, Server}),
    {dspawn, Server} ! stop,
    receive {'DOWN', Watched, _, _, Why} -> Why after 10000 -> timeout end.
