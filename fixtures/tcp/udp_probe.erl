-module(udp_probe).
-export([active/1, host_name/0, open/0, unsupported/1]).

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
unsupported(command) ->
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    erlang:port_command(P, <<"x">>);
unsupported(accept) ->
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    [1] = control(P, 1, [1, 2, 2]),
    control(P, 26, [0, 0, 0, 0]).

open_with(Bytes) ->
    P = open_port({spawn_driver, "udp_inet"}, [binary]),
    control(P, 1, Bytes).
