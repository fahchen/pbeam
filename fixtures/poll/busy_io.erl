-module(busy_io).
-export([main/0, port_root/0, spin/0, flush/0]).

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

%% A busy process holds a socket whose send buffer is full while the peer
%% drains it. The ERTS poll thread reports the write fd while the process
%% runs, so the sender finishes before the busy loop ends
%% (erl_process.c:3339-3372). The first line is the listen port for the
%% client of the test, which drains the socket and then sends one byte.
flush() ->
    {ok, L} = gen_tcp:listen(0, [binary, {active, false}, {ip, {127, 0, 0, 1}}]),
    {ok, Number} = inet:port(L),
    io:format("~w~n", [Number]),
    {ok, S} = gen_tcp:accept(L, 5000),
    Self = self(),
    Data = binary:copy(<<"x">>, 1 bsl 20),
    spawn(fun() -> [ok = gen_tcp:send(S, Data) || _ <- lists:seq(1, 8)], Self ! sent end),
    busy(erlang:monotonic_time(millisecond) + 3000),
    Result = receive sent -> sent_while_busy after 0 -> sent_after_busy end,
    receive sent -> ok after 10000 -> ok end,
    {ok, _} = gen_tcp:recv(S, 1, 10000),
    io:format("~w~n", [Result]).

busy(T) ->
    case erlang:monotonic_time(millisecond) >= T of
        true -> ok;
        false -> busy(T)
    end.
