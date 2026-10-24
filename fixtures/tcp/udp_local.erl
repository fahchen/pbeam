-module(udp_local).
-export([run/0]).

-define(A, <<"/tmp/pbeam_udp_local_a">>).
-define(B, <<"/tmp/pbeam_udp_local_b">>).
-define(C, <<"/tmp/pbeam_udp_local_c">>).

%% The calls that local_udp gives to prim_inet on `local` datagram
%% sockets (local_udp.erl:71-86): OPEN with the family 5, BIND, NAME, a
%% send to a `local` address, an active and a passive receive, CONNECT,
%% PEER, a send to the peer, and a passive and an active receive from an unbound socket
%% (inet_drv.c:14585-14595, 1945-1948, 4915-4936). The test removes the
%% files of the names before and after each run.
run() ->
    {ok, S} = bound(?A, [binary, {active, false}]),
    {ok, T} = bound(?B, [binary, {active, true}]),
    Name = prim_inet:sockname(S),
    Sent = prim_inet:sendto(S, {local, ?B}, [], <<"hi">>),
    Active = receive {udp, T, F, P, D} -> {udp, F, P, D} after 1000 -> none end,
    ok = prim_inet:sendto(T, {local, ?A}, [], <<"yo">>),
    Passive = prim_inet:recvfrom(S, 0, 1000),
    {ok, U} = bound(?C, [list, {active, false}]),
    NotConnected = prim_inet:peername(U),
    Connect = prim_inet:connect(U, {local, ?A}, 0),
    Peer = prim_inet:peername(U),
    ok = prim_inet:sendto(U, {local, <<>>}, [], "zz"),
    FromPeer = prim_inet:recvfrom(S, 0, 1000),
    ok = prim_inet:sendto(S, {local, ?C}, [], <<"li">>),
    List = prim_inet:recvfrom(U, 0, 1000),
    {ok, V} = prim_inet:open(udp, local, dgram),
    Unbound = prim_inet:sockname(V),
    ok = prim_inet:sendto(V, {local, ?A}, [], "q"),
    FromUnbound = prim_inet:recvfrom(S, 0, 1000),
    ok = prim_inet:sendto(V, {local, ?B}, [], "r"),
    ActiveUnbound = receive {udp, T, F2, P2, D2} -> {udp, F2, P2, D2} after 1000 -> none end,
    [prim_inet:close(X) || X <- [S, T, U, V]],
    [Name, Sent, Active, Passive, NotConnected, Connect, Peer, FromPeer, List, Unbound, FromUnbound, ActiveUnbound].

bound(Path, Options) ->
    {ok, S} = prim_inet:open(udp, local, dgram),
    ok = prim_inet:setopts(S, Options),
    {ok, _} = prim_inet:bind(S, {local, Path}, 0),
    {ok, S}.
