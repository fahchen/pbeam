-module(tcp_probe).
-export([control/0, data/0, enabled/0, linger/0, unsupported/1]).

%% The requests of the tcp_inet driver that prim_inet sends for
%% gen_tcp:listen/2, inet:sockname/1, inet:getopts/2, and gen_tcp:close/1
%% with the listen options of Thousand Island (prim_inet.erl:85-123,
%% 175-275; docs/tcp-socket-design.md).
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

%% The reply bytes of each request of the tcp_inet driver.
control() ->
    process_flag(trap_exit, true),
    P = open_port({spawn_driver, "tcp_inet"}, [binary]),
    C = fun(Request, Bytes) -> control(P, Request, Bytes) end,
    Before = [C(5, []), C(27, [0, 5]), C(6, [1, 0, 0, 127, 0, 0, 1]), C(6, [1]),
              C(7, [30,0,0,0,1, 35,0,0,0,1, 0,0,0,0,1, 10,0,0,0,1, 24,0,0,0,1]),
              C(8, [24, 22, 30, 35, 0, 10, 3]), C(7, [])],
    Open = [C(1, [1, 1]), C(1, [9, 1, 1]), C(1, [1, 2, 1]), C(1, [1, 1, 9]),
            C(1, [1, 1, 1]), C(1, [1, 1, 0])],
    Options = [C(7, [24,0,0,0,1, 22,0,0,0,2, 30,0,0,117,48, 35,0,0,0,1,
                     0,0,0,0,0, 10,0,0,0,0, 99]),
               C(8, <<24, 22, 30, 35, 0, 10, 3>>),
               length(C(8, lists:duplicate(13107, 24))),
               C(8, lists:duplicate(13108, 24)),
               length(C(8, lists:duplicate(7281, 3))),
               C(8, lists:duplicate(7282, 3))],
    Unbound = C(5, []),
    Bind = [C(6, [1, 0, 0]), C(6, [9, 0, 0]), C(6, [])],
    [1, P1, P0] = C(6, [[1], <<0, 0>>, 127, 0, 0, 1]),
    Name = C(5, []) =:= [1, 1, P1, P0, 127, 0, 0, 1],
    Listen = [C(27, [0]), C(27, <<0, 5>>), C(6, [1, 0, 0, 127, 0, 0, 1]), C(1, [1, 1, 1])],
    Subscribe = [C(24, [1]), C(24, [1, 1]), C(24, []), C(24, [2]),
                 C(24, lists:duplicate(13107, 1)) =:= [1 | lists:append(lists:duplicate(13107, [1, 0, 0, 0, 0]))],
                 C(24, lists:duplicate(13108, 1))],
    Q = open_port({spawn_driver, "tcp_inet"}, [binary]),
    Used = [control(Q, 1, [1, 1, 1]), control(Q, 6, [1, P1, P0, 127, 0, 0, 1])],
    true = port_close(Q),
    Badarg = [control(Q, 5, []), control(P, -1 - ?MAGIC, []), control(P, 1 bsl 32 - ?MAGIC, []),
              control(P, 5, foo), control(P, 5, [256]), control(foo, 5, [])],
    %% A wait with an open tcp_inet port.
    receive after 1 -> ok end,
    true = port_close(P),
    [Before, Open, Options, Unbound, Bind, P1 * 256 + P0 > 0, Name, Listen, Subscribe, Used,
     Badarg, shared(), flush()].

%% A bind of a port that is not 0: with reuseaddr, a second socket binds
%% the wildcard address on the port of a first socket that does not
%% listen, which keeps the port, and the reply gives that port.
shared() ->
    Open = fun() ->
                   S = open_port({spawn_driver, "tcp_inet"}, [binary]),
                   [1] = control(S, 1, [1, 1, 1]),
                   [1] = control(S, 7, [0, 0, 0, 0, 1]),
                   S
           end,
    S1 = Open(),
    [1, P1, P0] = control(S1, 6, [1, 0, 0, 127, 0, 0, 1]),
    S2 = Open(),
    Result = control(S2, 6, [1, P1, P0, 0, 0, 0, 0]) =:= [1, P1, P0],
    true = port_close(S2),
    true = port_close(S1),
    Result.

%% The GETOPTS reply of reuseaddr and nodelay when they are on: the value
%% of getsockopt, which macOS gives as the flag bit of the option (4) and
%% Linux as 1 (inet_drv.c:9727-9750).
enabled() ->
    P = open_port({spawn_driver, "tcp_inet"}, [binary]),
    [1] = control(P, 1, [1, 1, 1]),
    [1] = control(P, 7, [0,0,0,0,1, 10,0,0,0,1]),
    Reply = control(P, 8, [0, 10]),
    true = port_close(P),
    Reply.

%% A linger time: SETOPTS and GETOPTS of linger on (inet_drv.c:7360-7385,
%% 9749-9752).
linger() ->
    P = opened(),
    Reply = [control(P, 7, [3, 0, 0, 0, 1, 0, 0, 0, 5]), control(P, 8, [3])],
    true = port_close(P),
    Reply.

%% The port data, the port information, and the port names of a tcp_inet
%% port, and a port of the spawn driver, whose control rejects each command
%% other than 16#04c76a00 (sys_drivers.c:731-756).
data() ->
    process_flag(trap_exit, true),
    P = open_port({spawn_driver, "tcp_inet"}, [binary]),
    First = erlang:port_get_data(P),
    Set = erlang:port_set_data(P, {inet_tcp, [1, <<"x">>]}),
    Data = erlang:port_get_data(P),
    Info = [Item || {Key, _} = Item <- erlang:port_info(P),
                    Key =/= links, Key =/= id, Key =/= connected],
    true = port_close(P),
    Closed = [attempt(fun() -> erlang:port_set_data(P, x) end),
              attempt(fun() -> erlang:port_get_data(P) end),
              attempt(fun() -> erlang:port_set_data(foo, x) end),
              attempt(fun() -> erlang:port_get_data(self()) end)],
    Names = [name(open_port({spawn_driver, Name}, [binary])) || Name <- [<<"tcp_inet">>, tcp_inet]],
    S = open_port({spawn_executable, "/bin/cat"}, [binary]),
    Spawn = [erlang:port_get_data(S), erlang:port_set_data(S, cat), erlang:port_get_data(S),
             control(S, 5 - ?MAGIC, [])],
    true = port_close(S),
    [First, Set, Data, Info, Closed, Names, Spawn, flush()].

name(Port) ->
    {name, Name} = lists:keyfind(name, 1, erlang:port_info(Port)),
    true = port_close(Port),
    Name.

attempt(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.

flush() ->
    receive
        {'EXIT', Port, Reason} when is_port(Port) -> [{'EXIT', port, Reason} | flush()];
        Message -> [Message | flush()]
    after 0 -> []
    end.

%% Each part of the tcp_inet driver that PBeam does not run.
unsupported(udp) -> open_port({spawn_driver, "udp_inet"}, [binary]);
unsupported(list) -> open_port({spawn_driver, "tcp_inet"}, []);
unsupported(command) -> port_command(opened(), <<"x">>);
unsupported(inet6) -> control(open_port({spawn_driver, "tcp_inet"}, [binary]), 1, [2, 1, 1]);
unsupported(mptcp) -> control(open_port({spawn_driver, "tcp_inet"}, [binary]), 1, [1, 1, 4]);
unsupported(any) -> control(opened(), 6, [3, 0, 0]);
unsupported(dontroute) -> control(opened(), 7, [2, 0, 0, 0, 1]);
unsupported(recvttl) -> control(opened(), 7, [47, 0, 0, 0, 1]);
unsupported(linger) -> linger();
unsupported(active_n) -> control(opened(), 7, [22, 0, 0, 0, 3, 0, 1]);
unsupported(getopt) -> control(opened(), 8, [2]);
unsupported(connect) -> control(opened(), 3, []);
unsupported(policy) -> open_port({spawn_driver, "tcp_inet"}, [binary]);
unsupported(spawn_control) ->
    S = open_port({spawn_executable, "/bin/cat"}, [binary]),
    erlang:port_control(S, 16#04c76a00, <<0:512>>).

opened() ->
    P = open_port({spawn_driver, "tcp_inet"}, [binary]),
    [1] = control(P, 1, [1, 1, 1]),
    P.
