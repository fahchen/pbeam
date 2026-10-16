-module(tcp_accept).
-export([main/1]).

%% Accepts on one listen socket with several acceptors, as gen_tcp:accept/1,2
%% calls them: inet_tcp:accept/1,2 calls prim_inet:accept/2,3 with the
%% family options [tos, ttl, recvtos, recvttl] (inet_tcp.erl:233-249).
%% The listen socket has the options of Thousand Island and other options
%% that prim_inet:accept_opts/3 copies (prim_inet.erl:502-518).
%%
%% The probe writes "Port Count" to the file Path when a client of the
%% test must make connections up to Count, and waits until the client
%% writes Count to the file Path.ack after its connections. Each result gives the port of a
%% peer as {peer, P}, which the test compares with the port of its client.
%% The probe traps exits, as the process of `erl -eval` does.

-define(MAGIC, 16#03f1a300).
-define(FAMILY, [tos, ttl, recvtos, recvttl]).
-define(OPTIONS, [active, nodelay, keepalive, delay_send, priority, linger, reuseaddr,
                  tos, ttl, recvtos, recvttl, mode, send_timeout, send_timeout_close]).

main(Path) ->
    process_flag(trap_exit, true),
    Raw = raw(),
    {ok, L} = prim_inet:open(tcp, inet, stream),
    ok = prim_inet:setopts(L, [binary, {active, false}, {nodelay, true}, {reuseaddr, true},
                               {send_timeout, 30000}, {send_timeout_close, true},
                               {keepalive, true}, {delay_send, true}, {tos, 16}, {ttl, 33},
                               {recvtos, true}]),
    {ok, _} = prim_inet:bind(L, {127, 0, 0, 1}, 0),
    ok = prim_inet:listen(L, 1024),
    {ok, {_, Port}} = prim_inet:sockname(L),
    Listen = [prim_inet:getopts(L, ?OPTIONS), prim_inet:peername(L),
              prim_inet:accept(L, 30, ?FAMILY)],
    %% A1 waits first, and A2 times out while A1 waits.
    A1 = acceptor(L, infinity, ?FAMILY),
    A2 = acceptor(L, 40, []),
    X = acceptor(L, 5000, ?FAMILY),
    A3 = acceptor(L, 5000, ?FAMILY),
    Timeout = result(A2),
    %% X exits, so the second connection goes to A3.
    Ref = monitor(process, X),
    exit(X, kill),
    receive {'DOWN', Ref, process, X, killed} -> ok end,
    connect(Path, Port, 1),
    First = result(A1),
    connect(Path, Port, 2),
    Second = result(A3),
    %% A pending connection with no acceptor: the accept is at once.
    connect(Path, Port, 3),
    {ok, S} = prim_inet:accept(L, 0, ?FAMILY),
    Third = accepted(L, S),
    %% The close of the listen socket ends each accept that waits.
    A4 = acceptor(L, infinity, []),
    A5 = acceptor(L, 3000, ?FAMILY),
    ok = prim_inet:close(L),
    Closed = [result(A4), result(A5)],
    [Pid ! stop || Pid <- [A1, A3]],
    ok = prim_inet:close(S),
    %% The close of a port sends its link exit signal and its DOWN message,
    %% and then the stop of the driver ends the accept that waits
    %% (io.c:3922-3948, 3746).
    {ok, L2} = prim_inet:open(tcp, inet, stream),
    {ok, _} = prim_inet:bind(L2, {127, 0, 0, 1}, 0),
    ok = prim_inet:listen(L2, 5),
    _ = monitor(port, L2),
    {ok, _} = prim_inet:async_accept(L2, -1),
    true = erlang:port_close(L2),
    [Raw, Listen, Timeout, First, Second, Third, Closed, flush()].

%% The reply bytes of ACCEPT and PEER on a socket that does not listen.
raw() ->
    P = open_port({spawn_driver, "tcp_inet"}, [binary]),
    C = fun(Request, Bytes) -> erlang:port_control(P, Request + ?MAGIC, Bytes) end,
    Before = [C(26, [255, 255, 255, 255]), C(4, []), C(7, [9, 0, 0, 0, 16])],
    [1] = C(1, [1, 1, 1]),
    Open = [C(26, [255, 255, 255, 255]), C(4, []), C(8, [1, 8, 9, 31, 43, 46, 47]),
            C(7, [3, 0, 0, 0, 1])],
    [1, _, _] = C(6, [1, 0, 0, 127, 0, 0, 1]),
    [1] = C(27, [0, 5]),
    Listening = [C(26, [0, 0, 0]), C(26, [0, 0, 0, 0, 0])],
    true = port_close(P),
    [Before, Open, Listening].

acceptor(L, Time, Family) ->
    Parent = self(),
    Pid = spawn(fun() ->
                        Result = case prim_inet:accept(L, Time, Family) of
                                     {ok, S} -> accepted(L, S);
                                     Error -> Error
                                 end,
                        Parent ! {self(), Result},
                        receive stop -> ok end
                end),
    %% The acceptor waits in the queue before the next one starts.
    receive after 20 -> ok end,
    Pid.

accepted(L, S) ->
    {ok, {{127, 0, 0, 1}, P}} = prim_inet:peername(S),
    Info = erlang:port_info(S),
    Owner = [lists:keyfind(Key, 1, Info) =:= {Key, Value}
             || {Key, Value} <- [{connected, self()}, {links, [self()]}, {name, "tcp_inet"}]],
    [ok, is_port(S), {peer, P}, prim_inet:sockname(S) =:= prim_inet:sockname(L), Owner,
     prim_inet:getopts(S, ?OPTIONS)].

result(Pid) ->
    receive {Pid, Result} -> Result after 3000 -> missing end.

%% Ask the client of the test for connections up to Count, and wait until
%% the client has made them.
connect(Path, Port, Count) ->
    Script = "printf '%s %s\\n' \"$1\" \"$2\" > \"$3.tmp\" && mv \"$3.tmp\" \"$3\" && "
             "until [ \"$(cat \"$3.ack\" 2>/dev/null)\" = \"$2\" ]; do sleep 0.01; done",
    Shell = open_port({spawn_executable, "/bin/sh"},
                      [binary, exit_status,
                       {args, ["-c", Script, "sh", integer_to_list(Port),
                               integer_to_list(Count), Path]}]),
    receive {Shell, {exit_status, 0}} -> ok end,
    receive {'EXIT', Shell, normal} -> ok end.

flush() ->
    receive
        Message when is_tuple(Message) ->
            [[if is_port(E) -> port; is_pid(E) -> pid; is_reference(E) -> ref; true -> E end
              || E <- tuple_to_list(Message)] | flush()]
    after 0 -> []
    end.
