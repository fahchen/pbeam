-module(gen_tcp_connect).
-export([main/0, timeout/0, ipv6/0]).

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
    [ok = gen_tcp:close(X) || X <- [V6, V4]],
    io:format("~w~n", [[Names, Transfer, Only, Option, Tags, flush()]]).

flush() ->
    receive Message -> [Message | flush()] after 0 -> [] end.
