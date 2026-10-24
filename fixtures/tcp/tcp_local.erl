-module(tcp_local).
-export([control/0]).

%% The replies of `OPEN` with the TCP protocol, and of `BIND` with the `any`
%% and `loopback` tags, a short address, and a long address, on a `local` socket.
control() ->
    Open = fun() ->
        Port = open_port({spawn_driver, "tcp_inet"}, [binary]),
        [1] = port_control(Port, 16#03f1a301, [5, 1, 0]),
        Port
    end,
    Binds = [[3, 0, 0], [4, 0, 0], [5, 9, 1], [5, 120 | lists:duplicate(120, $a)]],
    Tcp = open_port({spawn_driver, "tcp_inet"}, [binary]),
    [port_control(Tcp, 16#03f1a301, [5, 1, 1]) | [port_control(Open(), 16#03f1a306, Bind) || Bind <- Binds]].
