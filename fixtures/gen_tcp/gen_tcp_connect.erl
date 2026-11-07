-module(gen_tcp_connect).
-export([main/0, timeout/0, ipv6/0, packet/0, line/0, read_ahead_options/0, read_ahead_receive/0]).

%% Connects IPv4 tcp_inet sockets to a local listen socket, as
%% gen_tcp:connect/3,4 call inet_tcp:connect/4 and prim_inet:connect/4
%% (prim_inet.erl:368-431), which runs INET_REQ_CONNECT
%% (inet_drv.c:12064-12141) and waits for the inet_async message of the
%% write select of the socket (inet_drv.c:14242-14288).

main() ->
    process_flag(trap_exit, true),
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}]),
    {ok, P} = inet:port(L),
    {ok, S} = gen_tcp:connect({127, 0, 0, 1}, P, [binary, {active, false}]),
    {ok, A} = gen_tcp:accept(L, 5000),
    {ok, {Ip, Local}} = inet:sockname(S),
    {ok, {PeerIp, PeerPort}} = inet:peername(S),
    {ok, {_, Remote}} = inet:peername(A),
    Names = [Ip, Local =:= Remote, PeerIp, PeerPort =:= P],
    Transfer = [gen_tcp:send(A, <<"hi">>), gen_tcp:recv(S, 0, 1000),
                gen_tcp:send(S, <<"yo">>), gen_tcp:recv(A, 0, 1000)],
    Again = [prim_inet:connect(S, {127, 0, 0, 1}, P),
             prim_inet:connect(S, {127, 0, 0, 1}, P, foo)],
    {ok, S2} = gen_tcp:connect({127, 0, 0, 1}, P, [binary, {active, true}], 5000),
    {ok, A2} = gen_tcp:accept(L, 5000),
    ok = gen_tcp:send(A2, <<"act">>),
    Active = receive {tcp, S2, Data} -> Data after 5000 -> missing end,
    [ok = gen_tcp:close(X) || X <- [S, A, S2, A2]],
    {ok, L2} = gen_tcp:listen(0, []),
    {ok, P2} = inet:port(L2),
    ok = gen_tcp:close(L2),
    Refused = [gen_tcp:connect({127, 0, 0, 1}, P2, []),
               gen_tcp:connect({127, 0, 0, 1}, P2, [], 1000)],
    ok = gen_tcp:close(L),
    io:format("~w~n", [[Names, Transfer, Again, Active, Refused, flush()]]).

%% A connect to an address of TEST-NET-1 (RFC 5737) waits until the timer
%% of the connect ends it (inet_drv.c:12569-12580). A second connect while
%% the first waits gives einval, and a close of the port ends the connect
%% with closed (inet_drv.c:11883-11885).
timeout() ->
    process_flag(trap_exit, true),
    Timeout = gen_tcp:connect({192, 0, 2, 1}, 9, [], 200),
    {ok, S} = prim_inet:open(tcp, inet, stream),
    {ok, S, Ref} = prim_inet:async_connect(S, {192, 0, 2, 1}, 9, infinity),
    Busy = prim_inet:connect(S, {192, 0, 2, 1}, 9, 0),
    true = erlang:port_close(S),
    Closed = receive {inet_async, S, Ref, Status} -> Status after 1000 -> missing end,
    Exits = [{Tag, is_port(Port), Reason} || {Tag, Port, Reason} <- flush()],
    io:format("~w~n", [[Timeout, Busy, Closed, Exits]]).

%% Connects IPv6 tcp_inet sockets (INET_AF_INET6, inet_drv.c:11974;
%% inet_set_address, inet_drv.c:4726-4745; inet_get_address,
%% inet_drv.c:4905-4913), reads and sets ipv6_v6only (inet_drv.c:7806-7823,
%% 9538-9552), and binds to the address tags any and loopback of both
%% families (inet_drv.c:4817-4875).
ipv6() ->
    process_flag(trap_exit, true),
    Loopback = {0, 0, 0, 0, 0, 0, 0, 1},
    {ok, L} = gen_tcp:listen(0, [inet6, binary, {active, false}, {ip, Loopback}]),
    {ok, P} = inet:port(L),
    {ok, S} = gen_tcp:connect(Loopback, P, [inet6, binary, {active, false}]),
    {ok, A} = gen_tcp:accept(L, 5000),
    {ok, {Ip, Local}} = inet:sockname(S),
    {ok, {PeerIp, PeerPort}} = inet:peername(S),
    {ok, {AcceptIp, Remote}} = inet:peername(A),
    Names = [inet:sockname(L) =:= {ok, {Loopback, P}}, Ip, Local =:= Remote,
             PeerIp, PeerPort =:= P, AcceptIp],
    Transfer = [gen_tcp:send(A, <<"hi">>), gen_tcp:recv(S, 0, 1000)],
    Only = [inet:getopts(L, [ipv6_v6only]), inet:getopts(A, [ipv6_v6only])],
    [ok = gen_tcp:close(X) || X <- [S, A, L]],
    {ok, V6} = gen_tcp:listen(0, [inet6, {ipv6_v6only, true}]),
    {ok, V4} = gen_tcp:listen(0, []),
    Option = [inet:getopts(V6, [ipv6_v6only]), inet:getopts(V4, [ipv6_v6only]),
              inet:setopts(V4, [{ipv6_v6only, true}])],
    Tags = [begin
                {ok, T} = gen_tcp:listen(0, Opts),
                {ok, {Addr, _}} = inet:sockname(T),
                ok = gen_tcp:close(T),
                Addr
            end || Opts <- [[inet6], [inet6, {ip, any}], [inet6, {ip, loopback}],
                            [{ip, any}], [{ip, loopback}]]],
    {ok, Short} = prim_inet:open(tcp, inet, stream),
    %% A tag any or loopback without a port gives einval (inet_drv.c:4821).
    ShortBind = [erlang:port_control(Short, 16#03f1a306, [Tag, 0]) || Tag <- [3, 4]],
    %% gen_tcp sends explicit addresses, so the tags run on raw ports.
    Raw = [begin
               {ok, R} = prim_inet:open(tcp, Family, stream),
               [1 | _] = erlang:port_control(R, 16#03f1a306, [Tag, 0, 0]),
               {ok, {RawAddr, _}} = inet:sockname(R),
               ok = gen_tcp:close(R),
               RawAddr
           end || Family <- [inet, inet6], Tag <- [3, 4]],
    Class = [inet:setopts(V6, [{tclass, 8}, {recvtclass, true}]),
             inet:getopts(V6, [tclass, recvtclass])],
    [ok = gen_tcp:close(X) || X <- [V6, V4, Short]],
    io:format("~w~n", [[Names, Transfer, Only, Option, Tags, ShortBind, Raw, Class, flush()]]).

flush() ->
    receive Message -> [Message | flush()] after 0 -> [] end.

%% Sends and receives with {packet, 1 | 2 | 4} between a socket with the
%% option and a raw socket, as tcp_send, tcp_sendv, tcp_remain, tcp_deliver,
%% and tcp_recv do (inet_drv.c:12967-13235, 13736-13885;
%% packet_parser.c:248-284, 483-494): the header of each send, one whole
%% packet for each receive, a split packet, two packets in one read, active
%% modes, packet_size, a buffer smaller than the header, and a change of the
%% option.
packet() ->
    process_flag(trap_exit, true),
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}]),
    {ok, P} = inet:port(L),
    Sizes = [packet(L, P, H) || H <- [1, 2, 4]],
    Two = <<2:32, "ok", 5:32, "hello">>,
    Limits = [limit(L, P, Options, Data)
              || {Options, Data} <- [{[{packet_size, 4}], Two}, {[{buffer, 2}], Two},
                                     {[{buffer, 2}, {packet_size, 8}], Two},
                                     {[{packet_size, 16#80000000}], <<16#7fffffff:32, "ab">>},
                                     {[{packet_size, 16#ffffffff}], <<16#fffffffc:32, "ab">>}]],
    Active = active_limit(L, P),
    ok = gen_tcp:close(L),
    io:format("~w~n", [[Sizes, Limits, Active, flush()]]).

packet(L, P, H) ->
    {ok, S} = gen_tcp:connect({127, 0, 0, 1}, P, [binary, {active, false}, {packet, H}]),
    {ok, A} = gen_tcp:accept(L, 5000),
    F = fun(Data) -> <<(byte_size(Data)):(H * 8), Data/binary>> end,
    Send = [inet:getopts(S, [packet, packet_size]), gen_tcp:send(S, <<"abc">>),
            gen_tcp:recv(A, H + 3, 1000), gen_tcp:send(S, [<<"d">>, "ef"]),
            gen_tcp:recv(A, H + 3, 1000), gen_tcp:send(S, <<>>), gen_tcp:recv(A, H, 1000)],
    ok = gen_tcp:send(A, F(<<"xyz">>)),
    ok = gen_tcp:send(A, binary:part(F(<<"split">>), 0, H + 2)),
    Receive = [gen_tcp:recv(S, 0, 1000), gen_tcp:recv(S, 0, 100), gen_tcp:recv(S, 3, 100)],
    ok = gen_tcp:send(A, binary:part(F(<<"split">>), H + 2, 3)),
    ok = gen_tcp:send(A, [F(<<"one">>), F(<<>>), F(<<"two">>), F(<<"three">>), F(<<"four">>)]),
    timer:sleep(50),
    Buffered = [gen_tcp:recv(S, 0, 1000), gen_tcp:recv(S, 0, 1000), gen_tcp:recv(S, 0, 1000)],
    ok = inet:setopts(S, [{active, once}, list]),
    Once = [message(), message(), inet:getopts(S, [active])],
    ok = inet:setopts(S, [{active, true}, binary]),
    Many = [message(), message()],
    ok = inet:setopts(S, [{active, false}]),
    ok = gen_tcp:send(A, [F(<<"to raw">>), F(<<"back">>)]),
    timer:sleep(50),
    Change = [inet:setopts(S, [{packet, raw}]), gen_tcp:recv(S, H + 1, 1000),
              inet:setopts(S, [{packet, H}]), gen_tcp:recv(S, 0, 300),
              inet:setopts(S, [{packet, 0}]), inet:getopts(S, [packet])],
    Long = case H of
               1 -> [gen_tcp:send(element(2, inet:setopts(S, [{packet, 1}]) =:= ok andalso {ok, S}),
                                  binary:copy(<<"x">>, 300)),
                     gen_tcp:recv(A, 1, 1000)];
               _ -> []
           end,
    %% A change of the packet type in active mode delivers the bytes of
    %% the buffer (inet_drv.c:7971-7981, 11132-11138).
    Switch = case H of
                 1 -> [];
                 _ -> [inet:setopts(S, [{packet, H}, {active, true}]), gen_tcp:send(A, <<0>>),
                       timer:sleep(50), message(), inet:setopts(S, [{packet, raw}]), message()]
             end,
    [ok = gen_tcp:close(X) || X <- [S, A]],
    [Send, Receive, Buffered, Once, Many, Change, Long, Switch].

%% A packet that packet_size does not allow, here the second packet, and a
%% buffer smaller than the header give emsgsize to a passive receive, and
%% the socket can then not read (inet_drv.c:12849-12890, 13007-13019,
%% 13167-13171, 13227-13228). A packet_size above INT_MAX gives lengths
%% that are negative or 0 as a C int (packet_parser.c:486-494).
limit(L, P, Options, Data) ->
    {ok, S} = gen_tcp:connect({127, 0, 0, 1}, P, [binary, {active, false}, {packet, 4} | Options]),
    {ok, A} = gen_tcp:accept(L, 5000),
    ok = gen_tcp:send(A, Data),
    timer:sleep(50),
    Result = [inet:getopts(S, [packet_size, buffer]), gen_tcp:recv(S, 0, 300),
              gen_tcp:recv(S, 0, 100), gen_tcp:send(S, <<"z">>), gen_tcp:recv(A, 0, 1000)],
    [ok = gen_tcp:close(X) || X <- [S, A]],
    Result.

%% Sends and receives with {packet, line}, as the SSH version exchange
%% of ssh_connection_handler reads the hello and each info line
%% (ssh_connection_handler.erl:617-628, 636-640; inet_drv.c:12967-13235,
%% 13736-13885; packet_parser.c:293-322): one line for each active message,
%% one line for each passive receive, a line split across reads, a line
%% that fills the buffer, and the packet_size limit.
line() ->
    process_flag(trap_exit, true),
    {ok, L} = gen_tcp:listen(0, [inet, binary, {active, false}]),
    {ok, P} = inet:port(L),
    {ok, S} = gen_tcp:connect({127, 0, 0, 1}, P,
                              [inet, binary, {active, false}, {packet, line}]),
    {ok, A} = gen_tcp:accept(L, 5000),
    Send = [inet:getopts(S, [packet, packet_size]),
            gen_tcp:send(S, <<"abc\n">>), gen_tcp:recv(A, 0, 1000),
            gen_tcp:send(S, [<<"a">>, "b"]), gen_tcp:recv(A, 0, 1000)],
    %% A send with {packet, line} adds no header: the peer gets the bytes
    %% as they are.
    ok = gen_tcp:send(A, <<"one\n">>),
    Receive = [gen_tcp:recv(S, 0, 1000), gen_tcp:recv(S, 0, 100),
               gen_tcp:recv(S, 0, 100)],
    %% A line split across reads waits for its delimiter.
    ok = gen_tcp:send(A, <<"tw">>),
    Split = [gen_tcp:recv(S, 0, 100)],
    ok = gen_tcp:send(A, <<"o\n">>),
    Split2 = Split ++ [gen_tcp:recv(S, 0, 1000)],
    %% A line that fills the buffer gives the bytes of the buffer, and the
    %% rest of the line stays for the next receive.
    ok = inet:setopts(S, [{buffer, 4}]),
    ok = gen_tcp:send(A, <<"abcdxyz\n">>),
    Full = [gen_tcp:recv(S, 0, 1000), gen_tcp:recv(S, 0, 1000)],
    %% A line above packet_size gives emsgsize, with a delimiter and
    %% without one (packet_parser.c:293-322). Each case needs a new
    %% socket, because a passive receive ends with emsgsize and the socket
    %% cannot read again (inet_drv.c:12849-12890).
    Limit = [line_limit(L, P, [{packet_size, 4}], <<"abcde\n">>),
             line_limit(L, P, [{packet_size, 4}], <<"abcde">>)],
    ok = inet:setopts(S, [{buffer, 9216}, {packet_size, 0}]),
    ok = gen_tcp:send(A, <<"x\n">>),
    After = [gen_tcp:recv(S, 0, 1000), gen_tcp:recv(S, 0, 100)],
    %% One line for each active message, and a change to active delivers
    %% the bytes of the buffer (inet_drv.c:7971-7981, 11132-11138).
    ok = gen_tcp:send(A, [<<"m1\n">>, <<"m2\n">>]),
    ok = inet:setopts(S, [{active, once}]),
    Once = [message(), message(), inet:getopts(S, [active])],
    [ok = gen_tcp:close(X) || X <- [S, A]],
    ok = gen_tcp:close(L),
    io:format("~w~n", [[Send, Receive, Split2, Full, Limit, After, Once, flush()]]).

%% A line above packet_size with the options `Options`: a fresh socket for
%% the case, because the receive ends with emsgsize (inet_drv.c:12849-12890,
%% 12973-12997).
line_limit(L, P, Options, Data) ->
    {ok, S} = gen_tcp:connect({127, 0, 0, 1}, P,
                              [inet, binary, {active, false}, {packet, line}
                               | Options]),
    {ok, A} = gen_tcp:accept(L, 5000),
    ok = gen_tcp:send(A, Data),
    Result = gen_tcp:recv(S, 0, 1000),
    [ok = gen_tcp:close(X) || X <- [S, A]],
    Result.

%% In active mode, a packet that packet_size does not allow stops the
%% delivery with no message (inet_drv.c:13118-13119). The next read event,
%% here the close of the peer, gives the error and the close as messages,
%% and the port exits with the reason emsgsize (inet_drv.c:12878-12886).
active_limit(L, P) ->
    {ok, _} = gen_tcp:connect({127, 0, 0, 1}, P, [binary, {active, true}, {packet, 2},
                                                   {packet_size, 1}]),
    {ok, A} = gen_tcp:accept(L, 5000),
    ok = gen_tcp:send(A, <<1:16, "a", 2:16, "bc">>),
    First = [message(), message()],
    ok = gen_tcp:close(A),
    First ++ [message(), message(), message()].

message() ->
    receive
        {tcp, _, Data} -> {tcp, Data};
        {tcp_error, _, Reason} -> {tcp_error, Reason};
        {tcp_closed, _} -> tcp_closed;
        {'EXIT', _, Reason} -> {'EXIT', Reason}
    after 300 -> missing
    end.

%% inet_drv.c:7184-7197, 9305-9314, 11829-11831.
read_ahead_options() ->
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}, {read_ahead, false}]),
    {ok, P} = inet:port(L),
    {ok, S} = gen_tcp:connect({127,0,0,1}, P, [binary, {active, false}]),
    {ok, A} = gen_tcp:accept(L),
    Before = [inet:getopts(L, [read_ahead]), inet:getopts(A, [read_ahead]),
              inet:getopts(S, [read_ahead])],
    Change = [inet:setopts(L, [{read_ahead, true}]), inet:getopts(L, [read_ahead]),
              inet:getopts(A, [read_ahead])],
    {ok, U} = gen_udp:open(0, [{active, false}]),
    Udp = [inet:getopts(U, [read_ahead]),
           inet:setopts(U, [{read_ahead, false}]), inet:getopts(U, [read_ahead]),
           inet:setopts(U, [{read_ahead, true}]), inet:getopts(U, [read_ahead])],
    [ok = gen_tcp:close(X) || X <- [L,S,A]],
    ok = gen_udp:close(U),
    io:format("~w~n", [[Before, Change, Udp]]).

%% Set active after receiving the first of two packets. Buffered packets are
%% delivered by SETOPTS itself (inet_drv.c:11131-11148, 12996-13005).
read_ahead_receive() ->
    Results = [read_ahead_receive(Option, Packet) || Option <- [false,true], Packet <- [1,2,4,line]],
    io:format("~w~n", [[Results, read_ahead_split()]]).

read_ahead_split() ->
    {ok, L} = gen_tcp:listen(0, [binary, {active,false}, {packet,4}, {read_ahead,false}]),
    {ok, P} = inet:port(L),
    {ok, S} = gen_tcp:connect({127,0,0,1}, P, [binary, {active,false}]),
    {ok, A} = gen_tcp:accept(L),
    ok = gen_tcp:send(S, <<0,0>>),
    Timeout = gen_tcp:recv(A, 0, 50),
    ok = gen_tcp:send(S, <<0,1,"x">>),
    Packet = gen_tcp:recv(A, 0, 1000),
    ok = inet:setopts(A, [{packet,raw}]),
    ok = gen_tcp:send(S, <<"raw">>),
    Raw = gen_tcp:recv(A, 0, 1000),
    [ok = gen_tcp:close(X) || X <- [S,A,L]],
    [Timeout,Packet,Raw].

read_ahead_receive(Option, Packet) ->
    {ok, L} = gen_tcp:listen(0, [binary, {active,false}, {packet,Packet}, {read_ahead,Option}]),
    {ok, P} = inet:port(L),
    {ok, S} = gen_tcp:connect({127,0,0,1}, P, [binary, {active,false}]),
    {ok, A} = gen_tcp:accept(L),
    Bytes = case Packet of
                1 -> <<1,"a",1,"b">>;
                2 -> <<0,1,"a",0,1,"b">>;
                4 -> <<0,0,0,1,"a",0,0,0,1,"b">>;
                line -> <<"a\nb\n">>
            end,
    ok = gen_tcp:send(S, Bytes),
    timer:sleep(30),
    First = gen_tcp:recv(A, 0, 1000),
    ok = inet:setopts(A, [{active,true}]),
    Immediate = receive {tcp,A,Data} -> {tcp,Data} after 0 -> missing end,
    Later = case Immediate of
                missing -> receive {tcp,A,Data2} -> {tcp,Data2} after 1000 -> missing end;
                _ -> done
            end,
    [ok = gen_tcp:close(X) || X <- [S,A,L]],
    {First,Immediate,Later}.
