-module(tcp_close).
-export([main/1, linger_zero/1, message_close/1]).

%% Closes accepted sockets from each side, as gen_tcp:close/1 calls
%% inet:tcp_close/1 and prim_inet:close/1 (gen_tcp.erl:867-870;
%% inet.erl:4578-4581; prim_inet.erl:175-275), with an empty and with a
%% queued output, by erlang:port_close/1, by the exit of the owner, and by
%% the close and the shutdown of the client.
%%
%% The probe writes "Step Command" to the file Path when the client of
%% the test must act, and waits until the client writes Step to the file
%% Path.ack. The commands are "connect Port", "close", "shutdown" (of the
%% write side), and "drain" (read until the end of file). The probe traps
%% exits, as the process of `erl -eval` does.

-define(SUBSCRIBE, 24 + 16#03f1a300).

main(Path) ->
    process_flag(trap_exit, true),
    {L, S1} = connection(Path),
    %% With an empty queue, SUBSCRIBE gives 0 and the port closes at once.
    ok = prim_inet:send(S1, "hi"),
    Empty = [prim_inet:getopts(S1, [linger]), erlang:port_control(S1, ?SUBSCRIBE, [1]),
             gen_tcp:close(S1), erlang:port_info(S1, connected)],
    step(Path, 2, "drain"),
    %% With queued output, the close waits for {empty_out_q, S} while the
    %% client reads, and then closes the port.
    S2 = accept(Path, L, 3),
    ok = prim_inet:send(S2, big()),
    Size = queued(S2),
    Parent = self(),
    _ = spawn_link(fun() -> Parent ! {closed, gen_tcp:close(S2)} end),
    step(Path, 4, "drain"),
    Queued = [Size, receive {closed, R2} -> R2 after 3000 -> missing end,
              erlang:port_info(S2, connected), lists:sort(flush())],
    %% erlang:port_close/1 with queued output gives the link exit signal and
    %% the DOWN message at once, and the port stays in the CLOSING state: it
    %% writes its queue before it closes the socket (io.c:3922-3948).
    %% erlang:port_info/2 and erlang:port_control/3 find it, the other port
    %% BIFs give badarg, and link/1 and monitor/2 give noproc.
    S3 = accept(Path, L, 5),
    Ref3 = monitor(port, S3),
    ok = prim_inet:send(S3, big()),
    T = fun(F) -> try F() catch Class:Reason -> {Class, Reason} end end,
    Closing = [erlang:port_close(S3),
               receive {'EXIT', S3, R3} -> R3 after 3000 -> missing end,
               receive {'DOWN', Ref3, port, S3, D3} -> D3 after 3000 -> missing end,
               erlang:port_info(S3, connected) =:= {connected, self()},
               erlang:port_info(S3, links), queued(S3),
               T(fun() -> erlang:port_command(S3, <<0, 0>>) end),
               T(fun() -> erlang:port_close(S3) end),
               T(fun() -> erlang:port_connect(S3, self()) end),
               T(fun() -> erlang:port_get_data(S3) end),
               link(S3), receive {'EXIT', S3, L3} -> L3 after 3000 -> missing end],
    Mref3 = monitor(port, S3),
    Monitor = receive {'DOWN', Mref3, port, S3, M3} -> M3 after 3000 -> missing end,
    step(Path, 6, "drain"),
    Closed = [Closing, Monitor, erlang:port_info(S3, connected)],
    %% The exit of the owner closes the port with its reason, and the port
    %% writes its queue.
    S4 = accept(Path, L, 7),
    ok = prim_inet:send(S4, big()),
    Owner = spawn(fun() -> receive go -> exit(shutdown) end end),
    ok = gen_tcp:controlling_process(S4, Owner),
    Ref4 = monitor(port, S4),
    Owner ! go,
    Exited = receive {'DOWN', Ref4, port, S4, D4} -> D4 after 3000 -> missing end,
    step(Path, 8, "drain"),
    %% The client closes: a passive socket gives closed, and the close of
    %% the port gives ok.
    S5 = accept(Path, L, 9),
    step(Path, 10, "close"),
    Passive = [prim_inet:recv(S5, 0, 1000), gen_tcp:close(S5), erlang:port_info(S5, connected)],
    %% An active socket sends {tcp_closed, S} and ends the port;
    %% inet:tcp_close/1 takes the message.
    S6 = accept(Path, L, 11),
    Ref6 = monitor(port, S6),
    ok = prim_inet:setopts(S6, [{active, true}]),
    step(Path, 12, "close"),
    Active = [receive {'DOWN', Ref6, port, S6, D6} -> D6 after 3000 -> missing end,
              gen_tcp:close(S6)],
    %% The client shuts down its write side while the active port has
    %% queued output: the port sends {tcp_closed, S} and ends with the
    %% reason normal, and the end drops the queue (inet_drv.c:12836-12844;
    %% io.c:7208-7216).
    S7 = accept(Path, L, 13),
    Ref7 = monitor(port, S7),
    ok = prim_inet:send(S7, big()),
    ok = prim_inet:setopts(S7, [{active, true}]),
    step(Path, 14, "shutdown"),
    Shutdown = [receive {tcp_closed, S7} -> tcp_closed after 3000 -> missing end,
                receive {'DOWN', Ref7, port, S7, D7} -> D7 after 3000 -> missing end],
    step(Path, 15, "drain"),
    ok = prim_inet:close(L),
    [Empty, Queued, Closed, Exited, Passive, Active, Shutdown, lists:sort(flush())].

%% With linger {true, 0} and queued output, erlang:port_close/1 sets the
%% CLOSING state, tcp_inet_flush drops the queue, and the port terminates
%% with CLOSING set: the stop sends no message to the receive that waits,
%% and no {empty_out_q, S} (io.c:3659-3721, 3939-3948;
%% inet_drv.c:12708-12730).
linger_zero(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    ok = prim_inet:setopts(S, [{linger, {true, 0}}]),
    Linger = prim_inet:getopts(S, [linger]),
    ok = prim_inet:send(S, big()),
    Size = queued(S),
    {ok, Ref} = prim_inet:async_recv(S, 0, -1),
    Closed = erlang:port_close(S),
    Recv = receive {inet_async, S, Ref, R} -> R after 500 -> none end,
    ok = prim_inet:close(L),
    [Linger, Size, Closed, Recv, lists:sort(flush())].

%% A {Pid, close} message to a port with queued output sets
%% ERTS_PORT_SFLG_SEND_CLOSED, and the port sends {Port, closed} when it
%% terminates after the flush, not at once (io.c:3713-3719, 3753-3758).
message_close(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    ok = prim_inet:send(S, big()),
    Size = queued(S),
    S ! {self(), close},
    Early = receive {S, closed} -> early after 200 -> waiting end,
    step(Path, 2, "drain"),
    Late = receive {S, closed} -> closed after 3000 -> missing end,
    ok = prim_inet:close(L),
    [Size, Early, Late, lists:sort(flush())].

big() -> double(<<"0123456789abcdef">>, 19).

%% Subscribe to {empty_out_q, S} with INET_REQ_SUBSCRIBE, and return true
%% when the queue holds bytes (prim_inet.erl:1296-1304).
queued(S) ->
    [1, 1 | Size] = erlang:port_control(S, ?SUBSCRIBE, [1]),
    Size =/= [0, 0, 0, 0].

connection(Path) ->
    {ok, L} = prim_inet:open(tcp, inet, stream),
    ok = prim_inet:setopts(L, [binary, {active, false}]),
    {ok, _} = prim_inet:bind(L, {127, 0, 0, 1}, 0),
    ok = prim_inet:listen(L, 5),
    {L, accept(Path, L, 1)}.

accept(Path, L, Step) ->
    {ok, {_, Port}} = prim_inet:sockname(L),
    step(Path, Step, ["connect ", integer_to_list(Port)]),
    {ok, S} = prim_inet:accept(L, 5000, []),
    S.

double(Binary, 0) -> Binary;
double(Binary, N) -> double(<<Binary/binary, Binary/binary>>, N - 1).

%% Ask the client of the test for Command, and wait until it has run it.
step(Path, Step, Command) ->
    Script = "printf '%s %s\\n' \"$1\" \"$2\" > \"$3.tmp\" && mv \"$3.tmp\" \"$3\" && "
             "until [ \"$(cat \"$3.ack\" 2>/dev/null)\" = \"$1\" ]; do sleep 0.01; done",
    Shell = open_port({spawn_executable, "/bin/sh"},
                      [binary, exit_status,
                       {args, ["-c", Script, "sh", integer_to_list(Step),
                               lists:flatten(Command), Path]}]),
    receive {Shell, {exit_status, 0}} -> ok end,
    receive {'EXIT', Shell, normal} -> ok end.

flush() ->
    receive
        Message when is_tuple(Message) ->
            [[if is_port(E) -> port; is_pid(E) -> pid; is_reference(E) -> ref; true -> E end
              || E <- tuple_to_list(Message)] | flush()]
    after 0 -> []
    end.
