-module(udp_probe).
-export([active/1, host_name/0, open/0, passive/0, port_zero/0, truncate/0, unsupported/1]).

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

%% The first datagram of an active socket on 127.0.0.1 and the port Port
%% (inet_drv.c:14619-14621). The test sends the datagrams. The result
%% gives the port of the sender as `true` when it is above 0.
active(Port) ->
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    C = fun(Request, Bytes) -> control(P, Request, Bytes) end,
    [1] = C(1, [1, 2, 2]),
    [1] = C(7, [22, [0, 0, 0, 1]]),
    [1 | _] = C(6, [1, [Port bsr 8, Port band 255], 127, 0, 0, 1]),
    receive
        {udp, P, Address, From, Data} -> {udp, port, Address, From > 0, Data}
    after 5000 -> timeout
    end.

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
