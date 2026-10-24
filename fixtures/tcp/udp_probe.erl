-module(udp_probe).
-export([active/0, active_options/0, connect/0, host_name/0, open/0, passive/0, port_zero/0, reconnect/0, active_waiter/0, refused_active/0, refused_passive/0, truncate/0, unsupported/1]).

%% The requests of the udp_inet driver that inet_config:set_hostname/0
%% sends through inet_udp:open(0, []), inet:gethostname/1, and
%% inet_udp:close/1 (inet_config.erl:223-231), as a trace on pinned ERTS
%% gives them.
%%
%% Each result gives a port number as `true` when it is above 0, and
%% replaces a port in a message with the atom port, because the numbers of
%% ERTS and PBeam differ. Each probe traps exits, as the process of
%% `erl -eval` does.

-define(MAGIC, 16#03f1a300).

control(Port, Request, Bytes) ->
    try erlang:port_control(Port, Request + ?MAGIC, Bytes)
    catch error:Reason -> {error, Reason}
    end.

%% The reply bytes of the requests of inet_config:set_hostname/0, in order.
host_name() ->
    process_flag(trap_exit, true),
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    C = fun(Request, Bytes) -> control(P, Request, Bytes) end,
    Replies = [C(7, []), C(1, [1, 2, [2]]), C(7, [22, [0, 0, 0, 1]])],
    [1, P1, P0] = C(6, [1, [0, 0], 0, 0, 0, 0]),
    Name = C(12, []),
    Rest = [C(8, [3, []]), C(24, [1])],
    true = erlang:port_close(P),
    Exit = receive {'EXIT', P, Reason} -> {'EXIT', port, Reason} after 100 -> none end,
    [Replies, P1 * 256 + P0 > 0, Name, Rest, Exit].

%% The messages of an active socket on 127.0.0.1 that sends to itself
%% (inet_drv.c:1551-1560, 4106-4160, 4215-4239, 6968-7000, 15361-15576):
%% `true`, `once`, `{active, N}` with `{udp_passive, S}`, a negative
%% count on a passive socket, a count above 32767, a change of `active`
%% with prim_inet:setopts/2, and `read_packets`. A datagram that comes
%% after `once` stays in the socket until the next change to active.
active() ->
    process_flag(trap_exit, true),
    Lo = {127, 0, 0, 1},
    {ok, S} = udp(),
    {ok, {_, P}} = prim_inet:sockname(S),
    Send = fun(Data) -> ok = prim_inet:sendto(S, Lo, P, Data) end,
    Next = fun() -> receive M -> message(M, S, P) after 200 -> none end end,
    ok = prim_inet:setopts(S, [{active, true}]),
    Send(<<"a">>),
    Send(<<"b">>),
    True = [Next(), Next()],
    ok = prim_inet:setopts(S, [{active, once}, list]),
    Send("c"),
    Send("d"),
    Once = [Next(), Next(), prim_inet:getopts(S, [active])],
    ok = prim_inet:setopts(S, [{active, 2}]),
    Two = [prim_inet:getopts(S, [active]), Next(), Send("e"), Next(), Next(), Next(),
           prim_inet:getopts(S, [active])],
    Negative = [prim_inet:setopts(S, [{active, -5}]), Next(), prim_inet:getopts(S, [active])],
    Count = [prim_inet:setopts(S, [{active, 32767}]), prim_inet:setopts(S, [{active, 1}]),
             prim_inet:getopts(S, [active]), prim_inet:setopts(S, [{active, -32767}]),
             prim_inet:getopts(S, [active]), Next()],
    ok = prim_inet:setopts(S, [{active, false}]),
    Send("f"),
    Passive = [Next(), prim_inet:recvfrom(S, 0, 1000) =/= none],
    Packets = [prim_inet:getopts(S, [read_packets]), prim_inet:setopts(S, [{read_packets, 0}]),
               prim_inet:setopts(S, [{read_packets, 1}]), prim_inet:getopts(S, [read_packets])],
    [Send(D) || D <- ["g", "h", "i"]],
    ok = prim_inet:setopts(S, [{active, true}]),
    Burst = [Next(), Next(), Next(), Next()],
    [True, Once, Two, Negative, Count, Passive, Packets, Burst].

%% The GETOPTS reply of `{active, N}` at the reply limit of 64 KiB: the
%% count takes 2 more bytes (inet_drv.c:9176-9184), so 13106 options
%% before it give einval and 13105 options fit. The result gives the
%% length of a reply that is not an error.
active_options() ->
    {ok, S} = udp(),
    ok = prim_inet:setopts(S, [{active, 1}]),
    [case control(S, 8, lists:duplicate(N, 24) ++ [22]) of
         [0 | Error] -> Error;
         Reply -> length(Reply)
     end || N <- [13106, 13105]].

%% A message of the socket S with the port P of the sender as `true` and
%% the socket as the atom socket, because the numbers of ERTS and PBeam
%% differ.
message({udp, S, Address, P, Data}, S, P) -> {udp, socket, Address, true, Data};
message({udp_passive, S}, S, _) -> {udp_passive, socket};
message(Other, _, _) -> Other.

%% The errors of OPEN, GETHOSTNAME, LISTEN, and PEER (inet_drv.c:11219-11233,
%% 11272-11300, 14577-14636, 14846-14858). PEER of a socket that is not
%% connected gives enotconn. A stream socket with the UDP protocol gives
%% the errno of socket/3 of the host. A datagram socket has no
%% send_timeout and send_timeout_close (inet_drv.c:7099-7120, 9236-9255).
open() ->
    process_flag(trap_exit, true),
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    C = fun(Request, Bytes) -> control(P, Request, Bytes) end,
    Before = [C(12, [1]), C(12, []) =:= C(12, []), C(27, [0, 5])],
    Open = [C(1, [1, 2]), C(1, [9, 2, 2]), C(1, [1, 9, 9]), C(1, [1, 2, 9]),
            C(1, [1, 1, 2]), C(1, [1, 2, 0]), C(1, [1, 2, 2])],
    After = [C(27, [0, 5]), C(27, []), C(7, [30,0,0,0,1, 35,0,0,0,1, 31,0,0,0,1]),
             C(8, [30, 35, 31]), C(4, [])],
    true = erlang:port_close(P),
    [Before, Open, After].

%% A request of the udp_inet driver that PBeam does not run.
unsupported(stream) -> open_with([1, 1, 0]);
unsupported(seqpacket) -> open_with([1, 3, 0]);
unsupported(sctp) -> open_with([1, 2, 3]);
unsupported(inet6) ->
    {ok, S} = udp(),
    command(S, [2, 0, 1 | lists:duplicate(16, 0)] ++ [0, 0, 0, 0]);
unsupported(recvtos) ->
    {ok, S} = udp(),
    prim_inet:setopts(S, [{recvtos, true}]);
unsupported(ancillary) ->
    {ok, S} = udp(),
    command(S, [1, 0, 1, 127, 0, 0, 1, 0, 0, 0, 1, 0]);
unsupported(short_count) ->
    {ok, S} = udp(),
    control(S, 7, [22, 0, 0, 0, 3]);
unsupported(disconnect) ->
    {ok, S} = udp(),
    control(S, 3, []);
unsupported(failed_connect) ->
    %% Linux rejects a connect to the broadcast address without SO_BROADCAST.
    {ok, S} = udp(),
    prim_inet:connect(S, {255, 255, 255, 255}, 9);
unsupported(accept) ->
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    [1] = control(P, 1, [1, 2, 2]),
    control(P, 26, [0, 0, 0, 0]).

open_with(Bytes) ->
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    control(P, 1, Bytes).

%% The output of Bytes after the caller tag, as prim_inet:sendto/4 sends
%% it (prim_inet.erl:652-699), and the reply.
command(S, Bytes) ->
    Ref = make_ref(),
    Tag = term_to_binary(Ref, [local]),
    true = erlang:port_command(S, [<<(byte_size(Tag)):16, Tag/binary>>, Bytes]),
    receive {inet_reply, S, Reply, Ref} -> Reply after 1000 -> none end.

%% The sender of a received datagram as {Address, true} when it is the
%% port Port, because the numbers of ERTS and PBeam differ.
sender({ok, {Address, Port, Data}}, Port) -> {ok, {Address, true, Data}};
sender({ok, {Address, Port}}, Port) -> {ok, {Address, true}};
sender(Other, _) -> Other.

%% The calls of gen_udp:send/4 and gen_udp:recv/2,3, which inet_udp gives
%% to prim_inet:sendto/4 and prim_inet:recvfrom/3, on a passive socket
%% on 127.0.0.1
%% (inet_drv.c:14968-14998, 15037-15316, 15361-15576): the data in binary
%% and list mode, an empty datagram, a timeout of 0 and of 30 ms, a
%% receive while another process waits (ealready), a datagram that comes
%% while a receive waits in the IO poll, the errors of the output and of
%% PACKET_REQ_RECV, and the GETSTAT counts.
passive() ->
    process_flag(trap_exit, true),
    Lo = {127, 0, 0, 1},
    {ok, S} = udp(),
    {ok, {_, P}} = prim_inet:sockname(S),
    Sent = prim_inet:sendto(S, Lo, P, <<"hi">>),
    Data = sender(prim_inet:recvfrom(S, 0, 1000), P),
    Timeouts = [prim_inet:recvfrom(S, 0, 0), prim_inet:recvfrom(S, 5, 30)],
    ok = prim_inet:sendto(S, Lo, P, <<>>),
    Empty = sender(prim_inet:recvfrom(S, 0), P),
    ok = prim_inet:setopts(S, [list]),
    ok = prim_inet:sendto(S, Lo, P, "ab"),
    List = sender(prim_inet:recvfrom(S, 0), P),
    Self = self(),
    spawn(fun() -> Self ! {waiter, sender(prim_inet:recvfrom(S, 0, 5000), P)} end),
    timer:sleep(50),
    Already = prim_inet:recvfrom(S, 0, 10),
    ok = prim_inet:sendto(S, Lo, P, "w"),
    Waited = receive {waiter, W} -> W after 1000 -> none end,
    Stat = prim_inet:getstat(S, [recv_oct, recv_cnt, recv_max, recv_avg, recv_dvi,
                                   send_oct, send_cnt, send_max, send_avg, send_pend]),
    Output = [command(S, Bytes) || Bytes <- [[], [9], [1, 0], [1, 0, 1, 127, 0, 0, 1, 0, 0],
                                            [1, 0, 1, 127, 0, 0, 1, 0, 0, 0, 2, 0]]],
    C = fun(Request, Bytes) -> erlang:port_control(S, Request + ?MAGIC, Bytes) end,
    Control = [C(60, [0, 0, 0, 0, 0, 0, 0]), C(7, [22, 0, 0, 0, 1]), C(60, [0, 0, 0, 0, 0, 0, 0, 0])],
    Unopened = open_port({spawn_driver, "udp_inet"}, [binary]),
    Closed = [command(Unopened, [1, 0, 1, 127, 0, 0, 1, 0, 0, 0, 0]),
              control(Unopened, 60, [0, 0, 0, 0, 0, 0, 0, 0])],
    true = erlang:port_command(Unopened, <<0>>),
    Exit = receive {'EXIT', Unopened, Reason} -> {'EXIT', port, Reason} after 1000 -> none end,
    [Sent, Data, Timeouts, Empty, List, Already, Waited, Stat, Output, Control, Closed, Exit].

%% A datagram longer than the buffer of 9216 bytes: the host drops the
%% rest (inet_drv.c:15469-15470). macOS sends at most 9216 bytes, so the
%% send gives emsgsize.
truncate() ->
    {ok, S} = udp(),
    {ok, {_, P}} = prim_inet:sockname(S),
    case prim_inet:sendto(S, {127, 0, 0, 1}, P, binary:copy(<<"x">>, 9300)) of
        ok ->
            {ok, {_, _, Data}} = prim_inet:recvfrom(S, 0, 1000),
            {ok, byte_size(Data), prim_inet:getstat(S, [recv_oct, send_oct])};
        Error ->
            Error
    end.

%% A passive binary socket on a port that the host selects, as
%% inet_udp:open/2 makes it without the inet_db table.
udp() ->
    {ok, S} = prim_inet:open(udp, inet, dgram),
    ok = prim_inet:setopts(S, [binary, {active, false}]),
    {ok, _} = prim_inet:bind(S, {0, 0, 0, 0}, 0),
    {ok, S}.

%% A send to the port 0 gives the errno of sendto/6 of the host
%% (inet_drv.c:15181, 15240-15253).
port_zero() ->
    {ok, S} = udp(),
    prim_inet:sendto(S, {127, 0, 0, 1}, 0, <<"x">>).

%% gen_udp:connect/3 and gen_udp:send/2, which inet_udp gives to
%% prim_inet:connect/3 and prim_inet:sendto/4 with {any, 0}
%% (inet_udp.erl:90-98), on passive sockets on 127.0.0.1
%% (inet_drv.c:14710-14842, 15142-15182): PEER before and after the
%% connect, a send without an address, a receive from the peer only, a
%% send to another address that goes to the peer, BIND of a connected
%% socket, the errors of CONNECT, and a send to {loopback, Port}.
connect() ->
    Lo = {127, 0, 0, 1},
    [{A, PA}, {B, PB}, {C, PC}] = [bound() || _ <- [a, b, c]],
    NotConnected = prim_inet:peername(A),
    Connect = prim_inet:connect(A, Lo, PB),
    Peer = sender(prim_inet:peername(A), PB),
    Sent = prim_inet:sendto(A, {any, 0}, [], <<"hi">>),
    Got = sender(prim_inet:recvfrom(B, 0, 1000), PA),
    ok = prim_inet:sendto(C, Lo, PA, <<"other">>),
    ok = prim_inet:sendto(B, Lo, PA, <<"peer">>),
    FromPeer = sender(prim_inet:recvfrom(A, 0, 1000), PB),
    Filtered = prim_inet:recvfrom(A, 0, 50),
    ok = prim_inet:sendto(A, Lo, PC, <<"to">>),
    ToPeer = [sender(prim_inet:recvfrom(B, 0, 1000), PA), prim_inet:recvfrom(C, 0, 50)],
    Bind = prim_inet:bind(A, {0, 0, 0, 0}, 0),
    Unopened = open_port({spawn_driver, "udp_inet"}, [binary]),
    Errors = [control(A, 3, [0]), control(A, 3, [0, 0, 0, 0, 1]), control(A, 3, [0, 0, 0, 0, 9, 0]),
              control(A, 3, [0, 0, 0, 0, 1, 0, 1]), control(Unopened, 3, [0, 0, 0, 0, 1, 0, 1, 127, 0, 0, 1])],
    {D, PD} = bound(),
    Loopback = [prim_inet:sendto(D, {loopback, PD}, [], <<"l">>), sender(prim_inet:recvfrom(D, 0, 1000), PD),
                command(D, [3, 0]), command(D, [4, 0, 1, 0, 0, 0])],
    [NotConnected, Connect, Peer, Sent, Got, FromPeer, Filtered, ToPeer, Bind, Errors, Loopback].

%% A send of a connected socket to a closed port: the ICMP error gives
%% econnrefused to the next receive of a passive socket, and
%% {udp_error, S, econnrefused} to the owner of an active socket
%% (inet_drv.c:15473-15489).
refused_passive() -> refused(false).

refused_active() -> refused(true).

refused(Active) ->
    {ok, S} = udp(),
    {Closed, P} = bound(),
    ok = prim_inet:close(Closed),
    ok = prim_inet:connect(S, {127, 0, 0, 1}, P),
    ok = prim_inet:sendto(S, {any, 0}, [], <<"x">>),
    ok = prim_inet:setopts(S, [{active, Active}]),
    case Active of
        false -> prim_inet:recvfrom(S, 0, 1000);
        true -> receive {udp_error, S, Reason} -> {udp_error, socket, Reason} after 1000 -> none end
    end.

%% A passive socket of udp/0 and its port.
bound() ->
    {ok, S} = udp(),
    {ok, {_, P}} = prim_inet:sockname(S),
    {S, P}.

%% CONNECT of a connected socket, which dissolves the association first
%% (inet_drv.c:14819-14836): a connect to the broadcast address, a failed
%% connect to the port 0, which xnu rejects, and a connect after it.
reconnect() ->
    {S, _} = bound(),
    {B, PB} = bound(),
    ok = prim_inet:connect(S, {127, 0, 0, 1}, 9),
    [prim_inet:connect(S, {255, 255, 255, 255}, 9), prim_inet:connect(S, {127, 0, 0, 1}, 0),
     prim_inet:peername(S), prim_inet:connect(S, {127, 0, 0, 1}, PB), sender(prim_inet:peername(S), PB),
     prim_inet:close(B)].

%% A connected passive socket with a receive that waits becomes active
%% once, and a datagram of the peer comes: the socket gives it to the owner,
%% because it is active (inet_drv.c:4335-4357). The waiter does not get
%% it; its end differs on PBeam (#1246).
active_waiter() ->
    {S, PS} = bound(),
    {B, PB} = bound(),
    ok = prim_inet:connect(S, {127, 0, 0, 1}, PB),
    Self = self(),
    spawn(fun() -> Self ! {waiter, prim_inet:recvfrom(S, 0, 300)} end),
    timer:sleep(50),
    ok = prim_inet:setopts(S, [{active, once}]),
    ok = prim_inet:sendto(B, {127, 0, 0, 1}, PS, <<"w">>),
    Owner = receive {udp, S, _, _, Data} -> {udp, Data} after 500 -> none end,
    Waiter = receive {waiter, {ok, _} = W} -> W after 0 -> none end,
    [Owner, Waiter].
