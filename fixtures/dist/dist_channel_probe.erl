%% The BIFs of a distribution channel with a process controller on an
%% alive node, with no other node (dist.c:4272-4720, 5127-5711).
-module(dist_channel_probe).
-compile(nowarn_deprecated_catch).
-export([main/0, net_kernel_exit/0, port_channel/0, no_mandatory/0, message/0,
         port_message/0, deliver_port/0, deliver_udp/0, input/0, reject/1,
         trace_token/0, link_op/0, atom_cache/0, output/0, port_output/0]).

main() ->
    %% net_kernel reports the dist_ctrlr message of a connection that it
    %% did not start (net_kernel.erl:1459-1461).
    logger:set_primary_config(level, none),
    M = mandatory(),
    Me = node(),
    show(new_connection, [catch erts_internal:new_connection(Me),
                          catch erts_internal:new_connection(1),
                          catch erts_internal:new_connection('a.b@c')]),
    {Id, Ref} = H = erts_internal:new_connection(zz@nohost),
    show(pending, [erts_internal:new_connection(zz@nohost) =:= H,
                   is_reference(Ref), nodes(known), nodes(hidden),
                   nodes(known, #{connection_id => true, node_type => true}),
                   erlang:dist_get_stat(H)]),
    show(notsup, [catch erlang:dist_ctrl_get_data(H),
                  catch erlang:dist_ctrl_get_data_notification(H),
                  catch erlang:dist_ctrl_get_opt(H, get_size),
                  catch erlang:dist_ctrl_set_opt(H, get_size, true),
                  catch erlang:dist_ctrl_put_data(H, <<>>),
                  catch erlang:dist_ctrl_input_handler(H, self()),
                  catch erlang:dist_ctrl_input_handler(foo, self()),
                  catch erlang:dist_ctrl_put_data(foo, <<>>),
                  catch erlang:dist_get_stat(foo),
                  catch erlang:dist_get_stat({Id + 1, Ref})]),
    show(abort, [catch erts_internal:abort_pending_connection(1, H),
                 catch erts_internal:abort_pending_connection(yy@nohost, H),
                 catch erts_internal:abort_pending_connection(zz@nohost, foo),
                 erts_internal:abort_pending_connection(zz@nohost, H),
                 erts_internal:abort_pending_connection(zz@nohost, H),
                 nodes(known)]),
    {Id2, Ref} = H2 = erts_internal:new_connection(zz@nohost),
    show(next, [(Id2 - Id) band 16#ffffff,
                erts_internal:abort_pending_connection(zz@nohost, H),
                nodes(known)]),
    show(create_errors,
         [catch erts_internal:create_dist_channel(1, self(), {M, 1}),
          catch erts_internal:create_dist_channel(Me, self(), {M, 1}),
          catch erts_internal:create_dist_channel(zz@nohost, foo, {M, 1}),
          catch erts_internal:create_dist_channel(zz@nohost, self(), {M}),
          catch erts_internal:create_dist_channel(zz@nohost, self(), foo),
          catch erts_internal:create_dist_channel(zz@nohost, self(), {-1, 1}),
          catch erts_internal:create_dist_channel(zz@nohost, self(), {1 bsl 64, 1}),
          catch erts_internal:create_dist_channel(zz@nohost, self(), {M, -1}),
          catch erts_internal:create_dist_channel(zz@nohost, self(), {M, 1 bsl 60}),
          catch erts_internal:create_dist_channel(xx@nohost, self(), {M, 1}),
          nodes(known)]),
    Dead = spawn(fun() -> ok end),
    DeadRef = monitor(process, Dead),
    receive {'DOWN', DeadRef, _, _, _} -> ok end,
    show(dead, [catch erts_internal:create_dist_channel(zz@nohost, Dead, {M, 1})]),
    Parent = self(),
    Ctrl = spawn(fun() -> controller(Parent) end),
    {message, Reply} = erts_internal:create_dist_channel(zz@nohost, Ctrl, {M, 1}),
    DH = receive {Reply, R} -> R end,
    show(connected, [DH =:= H2, nodes(hidden), nodes(connected), nodes(visible),
                     nodes(known), info(nodes(hidden, #{connection_id => true, node_type => true}), Id2),
                     erts_internal:abort_pending_connection(zz@nohost, DH),
                     erts_internal:new_connection(zz@nohost) =:= DH]),
    {message, Reply2} = erts_internal:create_dist_channel(zz@nohost, Ctrl, {M, 1}),
    show(twice, [receive {Reply2, R2} -> R2 end,
                 catch erlang:dist_ctrl_get_data(DH),
                 catch erlang:dist_ctrl_input_handler(DH, self())]),
    Visible = spawn(fun() -> receive stop -> ok end end),
    erts_internal:new_connection(uu@nohost),
    {message, Reply3} = erts_internal:create_dist_channel(uu@nohost, Visible, {M bor 1, 1}),
    show(visible, [is_tuple(receive {Reply3, R3} -> R3 end), nodes(), nodes(visible),
                   nodes(connected), nodes(hidden), nodes(known)]),
    VisibleRef = monitor(process, Visible),
    Visible ! stop,
    receive {'DOWN', VisibleRef, _, _, _} -> ok end,
    Ctrl ! {go, DH},
    receive {Ctrl, Results} -> show(controller, Results) end,
    show(handler, [erlang:dist_ctrl_put_data(DH, <<>>),
                   erlang:dist_ctrl_put_data(DH, []),
                   erlang:dist_ctrl_put_data(DH, [[], <<>>]),
                   catch erlang:dist_ctrl_put_data(DH, [foo]),
                   catch erlang:dist_ctrl_put_data(DH, <<1:1>>),
                   catch erlang:dist_ctrl_put_data(DH, foo),
                   erlang:dist_get_stat(DH)]),
    CtrlRef = monitor(process, Ctrl),
    Ctrl ! stop,
    receive {'DOWN', CtrlRef, _, _, _} -> ok end,
    show(down, [nodes(hidden), nodes(known),
                catch erlang:dist_ctrl_put_data(DH, <<>>),
                erlang:dist_get_stat(DH),
                erts_internal:abort_pending_connection(zz@nohost, DH)]),
    %% The caller itself is the controller.
    show(self, self_controller(M)).

%% The controller of zz@nohost runs the BIFs of a controller.
controller(Parent) ->
    receive
        {go, DH} ->
            Parent ! {self(),
                      [erlang:dist_ctrl_get_opt(DH, get_size),
                       erlang:dist_ctrl_set_opt(DH, get_size, true),
                       erlang:dist_ctrl_get_opt(DH, get_size),
                       catch erlang:dist_ctrl_set_opt(DH, get_size, yes),
                       catch erlang:dist_ctrl_set_opt(DH, other, true),
                       catch erlang:dist_ctrl_set_opt(DH, 1, true),
                       catch erlang:dist_ctrl_get_opt(DH, other),
                       erlang:dist_ctrl_get_data(DH),
                       erlang:dist_ctrl_get_data_notification(DH),
                       catch erlang:dist_ctrl_get_data(foo),
                       catch erlang:dist_ctrl_input_handler(DH, foo),
                       catch erlang:dist_ctrl_input_handler(setelement(1, DH, 0), Parent),
                       erlang:dist_ctrl_put_data(DH, <<>>),
                       catch erlang:dist_ctrl_put_data(DH, foo),
                       catch erlang:dist_ctrl_get_data({1, 2}),
                       erlang:dist_get_stat(DH),
                       erlang:dist_ctrl_input_handler(DH, Parent),
                       catch erlang:dist_ctrl_put_data(DH, <<>>)]},
            controller(Parent);
        stop ->
            ok
    end.

%% A process that controls ww@nohost after new_connection/1 calls
%% create_dist_channel/3 with itself as the controller.
self_controller(M) ->
    Parent = self(),
    spawn(fun() ->
                  erts_internal:new_connection(ww@nohost),
                  {ok, DH} = erts_internal:create_dist_channel(ww@nohost, self(), {M, 1}),
                  Parent ! {self_result, is_tuple(DH), lists:member(ww@nohost, nodes(hidden)),
                            catch erts_internal:create_dist_channel(vv@nohost, self(), {M, 1})}
          end),
    receive {self_result, A, B, C} -> [A, B, C] end.

%% The exit of net_kernel kills each process controller, ends each port
%% controller with net_kernel_terminated, and makes each pending
%% connection idle (dist.c:926-1001).
net_kernel_exit() ->
    logger:set_primary_config(level, none),
    process_flag(trap_exit, true),
    {ok, _} = net_kernel:start(dprobe, #{name_domain => shortnames}),
    M = mandatory(),
    erts_internal:new_connection(pp@nohost),
    erts_internal:new_connection(zz@nohost),
    Ctrl = spawn(fun() -> receive never -> ok end end),
    CtrlRef = monitor(process, Ctrl),
    {message, Reply} = erts_internal:create_dist_channel(zz@nohost, Ctrl, {M, 1}),
    receive {Reply, _} -> ok end,
    erts_internal:new_connection(qq@nohost),
    {ok, L} = gen_tcp:listen(0, []),
    {ok, _} = erts_internal:create_dist_channel(qq@nohost, L, {M, 1}),
    show(before, [nodes(known)]),
    ok = net_kernel:stop(),
    %% ERTS schedules the kill and the port exit (dist.c:975-990), so the
    %% order of zz@nohost and qq@nohost in nodes(known) is not fixed.
    show(stopped, [receive {'DOWN', CtrlRef, _, _, Reason} -> Reason end,
                   receive {'EXIT', L, PortReason} -> PortReason end,
                   node(), nodes(known) -- [qq@nohost],
                   lists:member(qq@nohost, nodes(known)),
                   erts_internal:create_dist_channel(zz@nohost, self(), {M, 1})]).

%% A tcp_inet port controls a connection (dist.c:5269-5336), and the exit
%% of the port ends it (io.c:3910-3916).
port_channel() ->
    logger:set_primary_config(level, none),
    M = mandatory(),
    erts_internal:new_connection(zz@nohost),
    Spawn = open_port({spawn, "cat"}, []),
    {ok, Udp} = gen_udp:open(0),
    {ok, Closed} = gen_tcp:listen(0, []),
    ok = gen_tcp:close(Closed),
    {ok, L} = gen_tcp:listen(0, []),
    show(port_errors, [erts_internal:create_dist_channel(zz@nohost, Spawn, {M, 1}),
                       erts_internal:create_dist_channel(zz@nohost, Udp, {M, 1}),
                       erts_internal:create_dist_channel(zz@nohost, Closed, {M, 1}),
                       erts_internal:create_dist_channel(yy@nohost, L, {M, 1}),
                       catch erlang:port_command(Udp, <<>>, [force]),
                       nodes(known)]),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, L, {M, 1}),
    erts_internal:new_connection(xx@nohost),
    show(port_up, [nodes(hidden), erlang:dist_get_stat(DH),
                   erts_internal:create_dist_channel(xx@nohost, L, {M, 1}),
                   erts_internal:create_dist_channel(zz@nohost, L, {M, 1}),
                   catch erlang:dist_ctrl_put_data(DH, <<>>),
                   catch erlang:dist_ctrl_get_data(DH),
                   catch erlang:dist_ctrl_input_handler(DH, self()),
                   inet:getopts(L, [deliver]),
                   inet:setopts(L, [{deliver, port}]),
                   inet:getopts(L, [deliver])]),
    ok = gen_tcp:close(L),
    timer:sleep(100),
    show(port_down, [nodes(hidden), nodes(known), erlang:dist_get_stat(DH)]).

%% Flags without each mandatory flag.
no_mandatory() ->
    erts_internal:new_connection(zz@nohost),
    erts_internal:create_dist_channel(zz@nohost, self(), {0, 1}).

%% A packet with data on a connection.
message() ->
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {mandatory(), 1}),
    erlang:dist_ctrl_put_data(DH, <<112>>).

%% A packet with data on a port controller.
port_message() ->
    {ok, L} = gen_tcp:listen(0, [binary]),
    {ok, Port} = inet:port(L),
    {ok, C} = gen_tcp:connect({127, 0, 0, 1}, Port, [binary, {packet, 4}]),
    {ok, A} = gen_tcp:accept(L),
    erts_internal:new_connection(zz@nohost),
    {ok, _} = erts_internal:create_dist_channel(zz@nohost, A, {mandatory(), 1}),
    ok = inet:setopts(A, [{active, true}, {deliver, port}, {packet, 4}]),
    ok = gen_tcp:send(C, <<112>>),
    timer:sleep(1000).

%% `{deliver, port}` on a tcp_inet port that is not a controller.
deliver_port() ->
    {ok, L} = gen_tcp:listen(0, [binary]),
    {ok, Port} = inet:port(L),
    {ok, C} = gen_tcp:connect({127, 0, 0, 1}, Port, [binary]),
    {ok, A} = gen_tcp:accept(L),
    ok = inet:setopts(A, [{active, true}, {deliver, port}]),
    ok = gen_tcp:send(C, <<"hi">>),
    receive Message -> io:format("~p~n", [Message]) end.

%% `{deliver, port}` on an active udp_inet port.
deliver_udp() ->
    {ok, U} = gen_udp:open(0, [binary, {active, true}, {deliver, port}]),
    {ok, Port} = inet:port(U),
    ok = gen_udp:send(U, {127, 0, 0, 1}, Port, <<"hi">>),
    receive Message -> io:format("~p~n", [Message]) end.

%% Packets with data to a process controller and to a port controller
%% go to a pid and a name, or are dropped (dist.c:2463-2553).
input() ->
    logger:set_primary_config(level, none),
    M = mandatory(),
    register(dinput, self()),
    Dead = spawn(fun() -> ok end),
    DeadRef = monitor(process, Dead),
    receive {'DOWN', DeadRef, _, _, _} -> ok end,
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {M, 1}),
    From = remote_pid(zz@nohost),
    Packets = fun(Tag) ->
                      [packet({2, '', self()}, {Tag, send}),
                       packet({22, From, self()}, {Tag, send_sender}),
                       packet({6, From, '', dinput}, {Tag, reg_send}),
                       packet({6, From, '', nobody}, {Tag, nobody}),
                       packet({2, '', Dead}, {Tag, dead}),
                       packet({22, From, From}, {Tag, remote}),
                       packet({2, '', Dead}, <<>>)]
              end,
    Put = [erlang:dist_ctrl_put_data(DH, P) || P <- Packets(process)],
    show(process, [Put, flush(), erlang:dist_get_stat(DH)]),
    {ok, L} = gen_tcp:listen(0, [binary]),
    {ok, Port} = inet:port(L),
    {ok, C} = gen_tcp:connect({127, 0, 0, 1}, Port, [binary, {packet, 4}]),
    {ok, A} = gen_tcp:accept(L),
    erts_internal:new_connection(yy@nohost),
    {ok, PH} = erts_internal:create_dist_channel(yy@nohost, A, {M, 1}),
    ok = inet:setopts(A, [{active, true}, {deliver, port}, {packet, 4}]),
    [ok = gen_tcp:send(C, P) || P <- Packets(port)],
    timer:sleep(200),
    show(port, [flush(), erlang:dist_get_stat(PH)]).

%% Messages to a pid, a name, and an alias of a connected node go to the
%% output queue of a process controller with the control message that the
%% flags permit (dist.c:1640-1702, 3646-3683, 4272-4746).
output() ->
    logger:set_primary_config(level, none),
    M = mandatory(),
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {M, 1}),
    P = remote_pid(zz@nohost),
    R = remote_ref(zz@nohost),
    show(empty, [erlang:dist_get_stat(DH), erlang:dist_ctrl_get_data(DH),
                 erlang:dist_ctrl_get_data_notification(DH),
                 erlang:dist_ctrl_get_data_notification(DH), flush()]),
    {nm, zz@nohost} ! hello,
    show(name, [erlang:dist_get_stat(DH), flush()]),
    P ! {a, <<1, 2, 3>>},
    R ! alias,
    P ! {big, binary:copy(<<7>>, 100)},
    show(sent, [erlang:dist_get_stat(DH), erlang:send(P, x, [noconnect]),
                erlang:send({nm, zz@nohost}, y, [nosuspend]), erlang:send(R, z),
                erlang:dist_ctrl_get_data_notification(DH), flush()]),
    show(get, [pkt(erlang:dist_ctrl_get_data(DH)), erlang:dist_get_stat(DH),
               erlang:dist_ctrl_set_opt(DH, get_size, true)]),
    Packets = [pkt(erlang:dist_ctrl_get_data(DH)) || _ <- lists:seq(1, 5)],
    show(get_size, Packets ++ [erlang:dist_get_stat(DH),
                    erlang:dist_ctrl_get_data_notification(DH), flush()]),
    P ! [1.5, fun lists:map/2, self(), make_ref(), #{a => 1}, <<1:3>>, -1, 1 bsl 70, "abc", {}],
    show(term, [flush(), pkt(erlang:dist_ctrl_get_data(DH)), erlang:dist_get_stat(DH)]),
    %% The flags of the other connections permit SEND_SENDER, ALIAS, and
    %% ALTACT_SIG.
    Flags = [{yy@nohost, M bor 16#80000}, {xx@nohost, M bor (1 bsl 35)},
             {ww@nohost, M bor (1 bsl 35) bor (1 bsl 37)}],
    show(flags, [flags_output(Node, F) || {Node, F} <- Flags]),
    Ctrl = spawn(fun() -> receive stop -> ok end end),
    erts_internal:new_connection(vv@nohost),
    {message, Reply} = erts_internal:create_dist_channel(vv@nohost, Ctrl, {M, 1}),
    VH = receive {Reply, VR} -> VR end,
    remote_pid(vv@nohost) ! queued,
    CtrlRef = monitor(process, Ctrl),
    Ctrl ! stop,
    receive {'DOWN', CtrlRef, _, _, _} -> ok end,
    show(down, [erlang:dist_get_stat(VH), remote_pid(vv@nohost) ! dropped, nodes(hidden)]).

%% The packets of a send and a priority send to a pid, a name, and an
%% alias on a connection with the flags Flags, which a new controller
%% takes (dist.c:1646-1700).
flags_output(Node, Flags) ->
    Parent = self(),
    Ctrl = spawn(fun() -> receive {go, D} -> Parent ! {self(), get_all(D)} end end),
    erts_internal:new_connection(Node),
    {message, Reply} = erts_internal:create_dist_channel(Node, Ctrl, {Flags, 1}),
    DH = receive {Reply, R} -> R end,
    remote_pid(Node) ! pid,
    {nm, Node} ! name,
    remote_ref(Node) ! alias,
    [erlang:send(To, Tag, [priority])
     || {To, Tag} <- [{remote_pid(Node), prio_pid}, {{nm, Node}, prio_name},
                      {remote_ref(Node), prio_alias}]],
    Ctrl ! {go, DH},
    receive {Ctrl, Packets} -> Packets end.

get_all(DH) ->
    case erlang:dist_ctrl_get_data(DH) of
        none -> [];
        Data -> [pkt(Data) | get_all(DH)]
    end.

%% A tcp_inet port controller writes each packet with its packet header
%% and gives no inet_reply; a write that fails closes the socket, and a
%% send to the closed socket clears its failed send
%% (dist.c:3844-3921, inet_drv.c:12660-12698).
port_output() ->
    logger:set_primary_config(level, none),
    M = mandatory(),
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}]),
    {ok, LPort} = inet:port(L),
    Connect = fun(Node, Options) ->
                      {ok, C} = gen_tcp:connect({127, 0, 0, 1}, LPort, [binary, {active, false} | Options]),
                      {ok, A} = gen_tcp:accept(L),
                      erts_internal:new_connection(Node),
                      {ok, DH} = erts_internal:create_dist_channel(Node, A, {M, 1}),
                      {C, A, DH}
              end,
    {C1, A1, DH1} = Connect(zz@nohost, [{packet, 4}]),
    ok = inet:setopts(A1, [{packet, 4}]),
    P1 = remote_pid(zz@nohost),
    P1 ! one,
    {nm, zz@nohost} ! two,
    timer:sleep(100),
    show(written, [pkt(gen_tcp:recv(C1, 0, 1000)), pkt(gen_tcp:recv(C1, 0, 1000)),
                   erlang:dist_get_stat(DH1), flush()]),
    %% The first write to a socket that the other side closed makes the
    %% other side reset it, so the next write fails and closes the socket;
    %% the send after it finds no connection.
    {C3, A3, DH3} = Connect(xx@nohost, []),
    ok = gen_tcp:close(C3),
    timer:sleep(100),
    P3 = remote_pid(xx@nohost),
    [begin P3 ! Tag, timer:sleep(100) end || Tag <- [first, reset, closed]],
    show(reset, [erlang:dist_get_stat(DH3), gen_tcp:send(A3, <<"x">>), gen_tcp:recv(A3, 0, 100),
                 flush()]).

%% Packets that ERTS rejects and ends the connection.
reject(N) ->
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {mandatory(), 1}),
    From = remote_pid(zz@nohost),
    Packet = case N of
                 1 -> <<112, 130, 97, 1>>;
                 2 -> <<112, 131, 68, 0>>;
                 3 -> <<112, 131, 97>>;
                 4 -> packet_only(foo);
                 5 -> packet_only({2, '', foo});
                 6 -> packet_only({2, '', self(), extra});
                 7 -> <<(packet_only({22, From, self()}))/binary, 131, 97>>;
                 8 -> packet_only({});
                 9 -> pid_reference_packet();
                 10 -> packet_only({-1, '', self()})
             end,
    erlang:dist_ctrl_put_data(DH, Packet).

%% A message with a reference of the local node that has the pid marker
%% bit (erl_bif_unique.h:106-109), which PBeam cannot hold.
pid_reference_packet() ->
    <<131, 90, Rest/binary>> = term_to_binary(make_ref()),
    Size = byte_size(Rest) - 12,
    <<Head:Size/binary, N0:32, N1:32, N2:32>> = Rest,
    <<(packet_only({2, '', self()}))/binary,
      131, 90, Head/binary, N0:32, (N1 bor (1 bsl 30)):32, N2:32>>.

%% A packet on a connection with DFLAG_DIST_HDR_ATOM_CACHE.
atom_cache() ->
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {mandatory() bor 16#2000, 1}),
    erlang:dist_ctrl_put_data(DH, packet_only({2, '', self()})).

%% A control message with a trace token.
trace_token() ->
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {mandatory(), 1}),
    erlang:dist_ctrl_put_data(DH, packet({12, '', self(), []}, m)).

%% An operation of another task (LINK).
link_op() ->
    erts_internal:new_connection(zz@nohost),
    {ok, DH} = erts_internal:create_dist_channel(zz@nohost, self(), {mandatory(), 1}),
    erlang:dist_ctrl_put_data(DH, packet_only({1, remote_pid(zz@nohost), self()})).

%% A pid of the node Node.
remote_pid(Node) ->
    Name = atom_to_binary(Node),
    binary_to_term(<<131, 88, 118, (byte_size(Name)):16, Name/binary, 1:32, 0:32, 1:32>>).

%% The element sizes, the control message, and the message of a packet,
%% with the pid of the caller as self, each local reference as ref, and
%% the node of each other pid and reference, which ERTS prints with the
%% index of its node.
pkt(none) -> none;
pkt({ok, Packet}) -> {ok, pkt(Packet)};
pkt({Size, Data}) -> {Size, pkt(Data)};
pkt(Data) when is_list(Data) -> {[byte_size(E) || E <- Data], pkt(iolist_to_binary(Data))};
pkt(<<112, Rest/binary>>) ->
    {Control, Used} = binary_to_term(Rest, [used]),
    <<_:Used/binary, Message/binary>> = Rest,
    {anon(Control), anon(binary_to_term(Message))}.

anon(Pid) when Pid =:= self() -> self;
anon(Pid) when is_pid(Pid) -> {pid, node(Pid)};
anon(Ref) when is_reference(Ref), node(Ref) =:= node() -> ref;
anon(Ref) when is_reference(Ref) -> {ref, node(Ref)};
anon(Tuple) when is_tuple(Tuple) -> list_to_tuple(anon(tuple_to_list(Tuple)));
anon([Head | Tail]) -> [anon(Head) | anon(Tail)];
anon(Term) -> Term.

%% A reference of the node Node.
remote_ref(Node) ->
    Name = atom_to_binary(Node),
    binary_to_term(<<131, 90, 0, 3, 118, (byte_size(Name)):16, Name/binary, 1:32, 1:32, 2:32, 3:32>>).

packet(Control, Message) ->
    <<(packet_only(Control))/binary, (term_to_binary(Message))/binary>>.

packet_only(Control) ->
    <<112, (term_to_binary(Control))/binary>>.

flush() ->
    receive M -> [M | flush()] after 0 -> [] end.

mandatory() ->
    element(3, erts_internal:get_dflags()).

info(Nodes, Id) ->
    [{Node, maps:update_with(connection_id, fun(C) -> C - Id end, Info)} || {Node, Info} <- Nodes].

show(Tag, Values) ->
    io:format("~p: ~p~n", [Tag, [strip(V) || V <- Values]]).

%% Keep the reason and the BIF of an exception, not the stack.
strip({'EXIT', {Reason, [{M, F, A, _} | _]}}) when is_list(A) -> {'EXIT', Reason, M, F, length(A)};
strip({'EXIT', {Reason, [{M, F, A, _} | _]}}) -> {'EXIT', Reason, M, F, A};
strip({'EXIT', Reason}) -> {'EXIT', Reason};
strip(Value) -> Value.
