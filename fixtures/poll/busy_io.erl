-module(busy_io).
-export([main/0, port_root/0, spin/0]).

%% A process that always can run shares the scheduler with the probe. The
%% probe still gets the input of a tcp_inet socket and of a spawn port, as
%% the ERTS poll thread waits for I/O while processes run
%% (erl_process.c:3339-3372). The first line is the listen port for the
%% client of the test, which sends "ab", waits, sends "cd", and then reads
%% until the probe closes.
main() ->
    Busy = spawn(?MODULE, spin, []),
    Tcp = tcp(),
    Port = port(),
    exit(Busy, kill),
    io:format("~w~n", [[Tcp, Port]]).

spin() -> spin().

%% A root of a run with more than one scheduler: the spawn port of the
%% root and the busy process share the scheduler of the root.
port_root() ->
    Busy = spawn(?MODULE, spin, []),
    Port = port(),
    exit(Busy, kill),
    Port.

tcp() ->
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}, {ip, {127, 0, 0, 1}}]),
    {ok, Number} = inet:port(L),
    io:format("~w~n", [Number]),
    {ok, S} = gen_tcp:accept(L, 5000),
    First = gen_tcp:recv(S, 0, 5000),
    ok = inet:setopts(S, [{active, once}]),
    Second = receive {tcp, S, Data} -> Data after 5000 -> timeout end,
    ok = gen_tcp:close(S),
    ok = gen_tcp:close(L),
    [First, Second].

port() ->
    P = open_port({spawn_executable, "/bin/echo"}, [binary, exit_status, {args, ["hi"]}]),
    [receive {P, M} -> M after 5000 -> timeout end || _ <- [1, 2]].
