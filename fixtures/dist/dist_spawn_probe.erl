%% Spawn processes on a server node with each spawn form, a link, a monitor,
%% a spawn request, the reply options, the group leader of a remote process,
%% and a node with no connection (bif.c:1027-1075,
%% erl_process.c:12440-12530, dist.c:5850-6260).
-module(dist_spawn_probe).
-export([server/0, client/1, leader/1, options/1, flags/1, demonitor/1,
         report/1, report_flags/1, wait/1, alias_wait/1, alias_order_sender/1,
         leader_target/1, boom/0]).

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
    CombinedRef = spawn_request(Server, dist_spawn_probe, wait, [self()],
                                [{monitor, [{tag, mine}]}, monitor]),
    CombinedPid = receive {spawn_reply, CombinedRef, ok, CP} -> CP after 5000 -> timeout end,
    ok = receive {started, CombinedPid} -> ok after 5000 -> timeout end,
    CombinedPid ! stop,
    CombinedTag = receive {Tag, CombinedRef, process, CombinedPid, normal} ->
                              {Tag, process, normal}
                  after 5000 -> timeout
                  end,
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
    PriorityRef = spawn_request(Server, dist_spawn_probe, wait, [self()],
                                [{monitor, [priority]}]),
    PriorityPid = receive {spawn_reply, PriorityRef, ok, PP} -> PP after 5000 -> timeout end,
    receive {started, PriorityPid} -> ok after 5000 -> timeout end,
    self() ! ordinary,
    PriorityPid ! stop,
    receive after 200 -> ok end,
    PriorityFirst = receive
                        {'DOWN', PriorityRef, process, PriorityPid, normal} -> down;
                        ordinary -> ordinary
                    after 5000 -> timeout
                    end,
    receive _ -> ok after 5000 -> timeout end,
    Alias = [spawn_alias(Server, Mode) || Mode <-
             [explicit_unalias, demonitor, reply_demonitor]],
    AliasLifetime = [spawn_alias_lifetime(Server, Mode, End) ||
                     Mode <- [explicit_unalias, demonitor, reply_demonitor],
                     End <- [down, demonitor]],
    AliasOrder = [spawn_alias_order(Server, Mode) || Mode <-
                  [explicit_unalias, demonitor, reply_demonitor]],
    process_flag(trap_exit, true),
    LinkRef = spawn_request(Server, dist_spawn_probe, wait, [self()], [{link, [priority]}]),
    LinkPid = receive {spawn_reply, LinkRef, ok, LP} -> LP after 5000 -> timeout end,
    receive {started, LinkPid} -> ok after 5000 -> timeout end,
    self() ! ordinary,
    LinkPid ! stop,
    receive after 200 -> ok end,
    LinkFirst = receive {'EXIT', LinkPid, normal} -> exit_signal; ordinary -> ordinary after 5000 -> timeout end,
    receive _ -> ok after 5000 -> timeout end,
    LeaderPid = spawn(Server, ?MODULE, leader_target, [self()]),
    receive {leader_ready, LeaderPid} -> ok after 5000 -> timeout end,
    LeaderConnected = group_leader(self(), LeaderPid),
    LeaderPid ! {check_leader, self()},
    LeaderSeen = receive {leader_value, LeaderPid, L1} -> L1 =:= self() after 5000 -> timeout end,
    Three = try erts_internal:group_leader(self(), LeaderPid, make_ref()) of
                V -> V catch error:badarg -> badarg end,
    true = erlang:disconnect_node(Server),
    LeaderPending = group_leader(group_leader(), LeaderPid),
    LeaderPid ! {check_leader, self()},
    LeaderSeenPending = receive {leader_value, LeaderPid, L2} -> L2 =:= group_leader() after 5000 -> timeout end,
    LeaderPid ! stop,
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
                {monitor_tag, MonitorTag}, {combined_monitor_tag, CombinedTag}, {reply_yes, ReplyYes},
                {no_connection, NoConnection}, {no_receiver, NoReceiver},
                {monitor_priority, PriorityFirst}, {monitor_alias, Alias},
                {alias_lifetime, AliasLifetime}, {alias_order, AliasOrder},
                {link_priority, LinkFirst},
                {remote_leader, {LeaderConnected, LeaderSeen, Three, LeaderPending, LeaderSeenPending}}]]),
    ok.

spawn_alias(Server, Mode) ->
    Ref = spawn_request(Server, dist_spawn_probe, alias_wait, [self()],
                        [{monitor, [{alias, Mode}]}]),
    Pid = receive {spawn_reply, Ref, ok, P} -> P after 5000 -> timeout end,
    receive {started, Pid} -> ok after 5000 -> timeout end,
    Pid ! {alias, Ref},
    Message = receive alias_message -> received after 5000 -> timeout end,
    Active = erlang:unalias(Ref),
    erlang:demonitor(Ref, [flush]),
    Pid ! stop,
    {Mode, Message, Active}.

spawn_alias_order(Server, Mode) ->
    Ref = spawn_request(Server, ?MODULE, alias_order_sender, [self()],
                        [{monitor, [{alias, Mode}]}]),
    Pid = receive {spawn_reply, Ref, ok, P} -> P after 5000 -> timeout end,
    ok = receive {started, Pid} -> ok after 5000 -> timeout end,
    Pid ! {send_order, Ref},
    Messages = alias_order_messages([]),
    erlang:demonitor(Ref, [flush]),
    erlang:unalias(Ref),
    Pid ! stop,
    {Mode, Messages}.

alias_order_messages(Acc) ->
    receive
        {ordered, done} -> lists:reverse(Acc);
        {ordered, Message} -> alias_order_messages([Message | Acc])
    after 5000 -> timeout
    end.

alias_order_sender(Parent) ->
    Parent ! {started, self()},
    receive {send_order, Ref} ->
        Parent ! {ordered, ordinary1},
        Ref ! {ordered, alias1},
        Parent ! {ordered, ordinary2},
        Ref ! {ordered, alias2},
        Parent ! {ordered, done}
    after 5000 -> timeout
    end,
    receive stop -> ok after 5000 -> timeout end.

spawn_alias_lifetime(Server, Mode, End) ->
    Ref = spawn_request(Server, dist_spawn_probe, wait, [self()],
                        [{monitor, [{alias, Mode}]}]),
    Pid = receive {spawn_reply, Ref, ok, P} -> P after 5000 -> timeout end,
    ok = receive {started, Pid} -> ok after 5000 -> timeout end,
    case End of
        down ->
            Pid ! stop,
            ok = receive {'DOWN', Ref, process, Pid, normal} -> ok after 5000 -> timeout end;
        demonitor ->
            true = erlang:demonitor(Ref),
            Pid ! stop
    end,
    Ref ! after_monitor,
    Message = receive after_monitor -> delivered after 200 -> dropped end,
    Active = erlang:unalias(Ref),
    {Mode, End, Message, Active}.

leader_target(Parent) ->
    Parent ! {leader_ready, self()},
    leader_target_loop().

leader_target_loop() ->
    receive
        {check_leader, Parent} -> Parent ! {leader_value, self(), group_leader()}, leader_target_loop();
        stop -> ok
    after 5000 -> ok
    end.

alias_wait(Parent) ->
    Parent ! {started, self()},
    receive {alias, Ref} -> Ref ! alias_message end,
    receive stop -> ok end.

%% The options of a spawn request set the start values of the priority, the
%% message queue data, and the async_dist flag of the new process
%% (`erl_process.c:12275-12320`, `12552-12558`).
flags(Server) ->
    true = net_kernel:connect_node(Server),
    Req = spawn_request(Server, dist_spawn_probe, report_flags, [self()],
                        [monitor, {priority, high}, {message_queue_data, off_heap},
                         {async_dist, true}]),
    Pid = receive {spawn_reply, Req, ok, P} -> P after 5000 -> timeout end,
    Info = receive {flags, I} -> I after 5000 -> timeout end,
    io:format("flags: ~p~n", [[{spawn, is_pid(Pid)}, {info, Info}]]),
    ok.

report_flags(Client) ->
    Client ! {flags, {process_info(self(), priority),
                      process_flag(message_queue_data, on_heap),
                      process_flag(async_dist, false)}},
    ok.

%% `demonitor/1` of a monitor that a spawn_monitor/4 started removes the
%% monitor of the process on the node (`bif.c:436-541`, `dist.c:2434-2459`).
demonitor(Server) ->
    true = net_kernel:connect_node(Server),
    {Pid, Ref} = spawn_monitor(Server, dist_spawn_probe, wait, [self()]),
    receive {started, Pid} -> ok after 5000 -> timeout end,
    true = erlang:demonitor(Ref),
    receive after 200 -> ok end,
    Observed = rpc:call(Server, erlang, process_info, [Pid, monitored_by]),
    Pid ! stop,
    io:format("demonitor: ~p~n", [Observed]),
    ok.

wait(Parent) ->
    Parent ! {started, self()},
    receive stop -> ok end.

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
