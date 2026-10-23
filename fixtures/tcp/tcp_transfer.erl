-module(tcp_transfer).
-export([main/1, reset/1, busy_close/1, unsupported/2]).

%% Receives and sends on an accepted socket in passive mode, as
%% gen_tcp:recv/3 and gen_tcp:send/2 call prim_inet:recv/3 and
%% prim_inet:send/2 (inet_tcp.erl; prim_inet.erl:575-624, 808-824), and
%% reads the counts of inet:getstat/1 (prim_inet:getstat/2).
%%
%% The probe writes "Step Command" to the file Path when the client of
%% the test must act, and waits until the client writes Step to the file
%% Path.ack. The commands are "connect Port", "send Text", "read Count",
%% "close", and "reset" (a close with linger 0). The probe traps exits,
%% as the process of `erl -eval` does.

-define(MAGIC, 16#03f1a300).
-define(STATS, [recv_oct, recv_cnt, recv_max, recv_avg, recv_dvi,
                send_oct, send_cnt, send_max, send_avg, send_pend]).

main(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    NotConnected = prim_inet:recv(L, 0),
    step(Path, 2, "send abcdefgh"),
    Receives = [prim_inet:recv(S, 3, 1000), prim_inet:recv(S, 0, 1000), prim_inet:recv(S, 0, 0)],
    step(Path, 3, "send xy"),
    %% The timeout keeps the bytes that the receive read.
    Partial = [prim_inet:recv(S, 5, 50), prim_inet:recv(S, 1, 1000), prim_inet:recv(S, 0, 1000)],
    %% A receive that waits for the client; a second receive gives ealready.
    [1, R1, R0] = erlang:port_control(S, 42 + ?MAGIC, [0, 0, 19, 136, 0, 0, 0, 2]),
    Already = prim_inet:recv(S, 1, 0),
    step(Path, 4, "send ok"),
    Ref = R1 * 256 + R0,
    Waited = receive {inet_async, S, Ref, Status} -> Status after 3000 -> missing end,
    ok = prim_inet:setopts(S, [list]),
    step(Path, 5, "send hi"),
    List = prim_inet:recv(S, 2, 1000),
    ok = prim_inet:setopts(S, [binary]),
    C = fun(Request, Bytes) ->
                try erlang:port_control(S, Request + ?MAGIC, Bytes) catch error:R -> R end
        end,
    Requests = [C(42, [0, 0, 0, 0, 0, 0, 0]), C(42, [0, 0, 0, 0, 4, 0, 0, 1]), C(11, [0]),
                C(11, lists:duplicate(13108, 1))],
    Sends = [prim_inet:send(S, "pong"), prim_inet:send(S, <<>>)],
    step(Path, 6, "read 4"),
    %% A send of more than the socket takes queues the rest, and a send
    %% while the queue holds 8 KiB or more waits for its reply until the
    %% client reads.
    Big = double(<<"0123456789abcdef">>, 19),
    Large = prim_inet:send(S, Big),
    {ok, [{send_pend, Pending}]} = prim_inet:getstat(S, [send_pend]),
    Parent = self(),
    Busy = spawn(fun() -> Parent ! {self(), prim_inet:send(S, <<"tail">>)} end),
    step(Path, 7, ["read ", integer_to_list(byte_size(Big) + 4)]),
    Waiting = receive {Busy, Sent} -> Sent after 3000 -> missing end,
    Stats = prim_inet:getstat(S, ?STATS),
    %% The client closes: a receive gives closed, then enotconn; a send
    %% gives closed, then enotconn.
    step(Path, 8, "close"),
    Closed = [prim_inet:recv(S, 0, 1000), prim_inet:recv(S, 0, 1000),
              prim_inet:send(S, "x"), prim_inet:send(S, "x"), prim_inet:getstat(S, ?STATS)],
    %% A header that is too short makes the port fail with einval.
    true = erlang:port_command(S, <<"x">>),
    Header = receive {'EXIT', S, Reason} -> Reason after 3000 -> missing end,
    %% The close of a port sends its link exit signal and its DOWN message,
    %% and then the stop of the driver ends the receive that waits
    %% (io.c:3922-3948, 3746).
    S2 = accept(Path, L, 9),
    _ = monitor(port, S2),
    [1, _, _] = erlang:port_control(S2, 42 + ?MAGIC, [0, 0, 19, 136, 0, 0, 0, 0]),
    true = erlang:port_close(S2),
    ok = prim_inet:close(L),
    [NotConnected, Receives, Partial, Already, Waited, List, Requests, Sends, Large, Pending > 0,
     Waiting, Stats, Closed, Header, flush()].

%% The client resets the connection while the port has queued output, and
%% resets a second connection before a send.
reset(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    ok = prim_inet:send(S, double(<<"0123456789abcdef">>, 19)),
    %% A send_timeout of 0 gives timeout at once when the queue is large.
    ok = prim_inet:setopts(S, [{send_timeout, 0}]),
    Timeout = prim_inet:send(S, "x"),
    step(Path, 2, "reset"),
    Queued = [prim_inet:send(S, "x"), prim_inet:send(S, "x"), prim_inet:recv(S, 0, 0),
              prim_inet:recv(S, 0, 0), prim_inet:getstat(S, [send_pend])],
    S2 = accept(Path, L, 3),
    step(Path, 4, "reset"),
    Direct = [prim_inet:send(S2, "x"), prim_inet:recv(S2, 0, 0), prim_inet:recv(S2, 0, 0),
              prim_inet:send(S2, "x")],
    [Timeout, Queued, Direct, flush()].

%% The client shuts down its write side while a send waits for the queue and a receive
%% waits for data: both get {error, closed}.
busy_close(Path) ->
    process_flag(trap_exit, true),
    {_, S} = connection(Path),
    ok = prim_inet:send(S, double(<<"0123456789abcdef">>, 19)),
    Parent = self(),
    Busy = spawn(fun() -> Parent ! {self(), prim_inet:send(S, <<"tail">>)} end),
    %% The wait lets Busy run until its send waits for the queue.
    receive after 50 -> ok end,
    [1, R1, R0] = erlang:port_control(S, 42 + ?MAGIC, [255, 255, 255, 255, 0, 0, 0, 0]),
    step(Path, 2, "shutdown"),
    Ref = R1 * 256 + R0,
    [receive {Busy, Sent} -> Sent after 3000 -> missing end,
     receive {inet_async, S, Ref, Status} -> Status after 3000 -> missing end].

%% Each part that PBeam does not run yet, on a connected socket.
unsupported(Path, Case) ->
    process_flag(trap_exit, true),
    {_, S} = connection(Path),
    case Case of
        length -> erlang:port_control(S, 42 + ?MAGIC, [0, 0, 0, 0, 255, 255, 255, 255]);
        header -> erlang:port_command(S, <<0, 1, 0>>);
        delay_send -> ok = prim_inet:setopts(S, [{delay_send, true}]), prim_inet:send(S, "x");
        send_timeout -> ok = prim_inet:setopts(S, [{send_timeout, 1000}]), queue(S),
                        prim_inet:send(S, "x");
        busy -> queue(S),
                Parent = self(),
                spawn(fun() -> Parent ! sent, prim_inet:send(S, "x") end),
                receive sent -> ok end,
                prim_inet:send(S, "y");
        term -> term_to_binary(S, [local])
    end.

queue(S) -> ok = prim_inet:send(S, double(<<"0123456789abcdef">>, 19)).

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
