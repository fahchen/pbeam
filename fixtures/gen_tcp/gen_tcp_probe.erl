-module(gen_tcp_probe).
-export([main/0]).

%% Runs the gen_tcp and inet calls of Thousand Island on a listen socket
%% with the listen options of Thousand Island (transports/tcp.ex:44-75)
%% and the inet family option, and writes the results with io:format/2.
%% gen_tcp:listen/2 calls inet_tcp:listen/2 (gen_tcp.erl:1219-1233), and
%% the family option selects inet_tcp without the inet_db table
%% (inet.erl:3283-3286).
%%
%% The probe writes the port of the listen socket on the first line. The
%% client of the test then connects twice. The client reads each text that the probe
%% sends and replies with the next data, so each recv reads one write of
%% the client.
main() ->
    {ok, L} = gen_tcp:listen(0, options()),
    {ok, {Address, Port}} = inet:sockname(L),
    Listen = [is_port(L), Address, Port > 0, gen_tcp:listen(Port, options()),
              inet:getopts(L, [mode, active, nodelay, reuseaddr, send_timeout, send_timeout_close]),
              erlang:port_get_data(L), gen_tcp:accept(L, 0)],
    io:format("~w~n", [Port]),
    {ok, S} = gen_tcp:accept(L),
    First = first(S),
    {ok, S2} = gen_tcp:accept(L, 5000),
    Second = [inet:getopts(S2, [mode, active, nodelay, send_timeout, send_timeout_close]),
              gen_tcp:send(S2, <<"bye">>), gen_tcp:recv(S2, 1, 5000), gen_tcp:close(S2),
              erlang:port_info(S2)],
    Closed = [gen_tcp:close(L), inet:sockname(L), inet:getopts(L, [active]), erlang:port_info(L)],
    {ok, Again} = gen_tcp:listen(Port, options()),
    ok = gen_tcp:close(Again),
    io:format("~w~n", [[Listen, First, Second, Closed, flush()]]).

%% Passive recv/2 and recv/3, {active, once}, and a move of the socket to
%% a handler that reads until the client closes.
first(S) ->
    Passive = [gen_tcp:send(S, <<"ready">>), gen_tcp:recv(S, 3), gen_tcp:send(S, <<"next">>),
               gen_tcp:recv(S, 0, 5000), gen_tcp:recv(S, 0, 0)],
    Once = [inet:setopts(S, [{active, once}]), gen_tcp:send(S, <<"once">>), message(S),
            inet:getopts(S, [active])],
    Parent = self(),
    Handler = spawn(fun() -> receive go -> Parent ! {self(), handler(S)} end end),
    Moved = gen_tcp:controlling_process(S, Handler),
    Handler ! go,
    Handled = receive {Handler, Result} -> Result after 5000 -> missing end,
    [Passive, Once, Moved, Handled, gen_tcp:close(S)].

handler(S) ->
    [inet:setopts(S, [{active, once}]), gen_tcp:send(S, <<"last">>), message(S),
     inet:setopts(S, [{active, once}]), message(S)].

options() ->
    [inet, {mode, binary}, {active, false}, {backlog, 1024}, {nodelay, true}, {send_timeout, 30000},
     {send_timeout_close, true}, {reuseaddr, true}].

message(S) ->
    receive
        {tcp, S, Data} -> {tcp, Data};
        {tcp_closed, S} -> tcp_closed;
        {tcp_error, S, Reason} -> {tcp_error, Reason}
    after 5000 -> missing
    end.

flush() ->
    receive Message -> [Message | flush()] after 0 -> [] end.
