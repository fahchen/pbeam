-module(timer_probe).
-export([receive_after/0, timeout_values/0, receive_limits/0, start_timer/0,
         cancel_timer/0, cancel_timer_1/0, receiver_exit/0, order/0, timer_limits/0,
         send_after/3, wait/0, id/1, send_after_timer/0, start_timer_4/0,
         send_after_4/0, read_timer/0]).

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant timeout or argument.
id(X) -> X.

%% Send Msg to Pid after Time milliseconds, with a receive timeout.
send_after(Pid, Time, Msg) ->
    receive after Time -> Pid ! Msg end.

wait() ->
    receive stop -> ok end.

%% Receive any message, or give none after Time milliseconds.
wait(Time) ->
    receive M -> M after Time -> none end.

%% A receive timeout. Total wait of at least 100 + 110 ms.
receive_after() ->
    Self = self(),
    A = receive _ -> got after ?MODULE:id(100) -> timeout end,
    spawn(?MODULE, send_after, [Self, 20, early]),
    B = receive early -> early after ?MODULE:id(1000) -> timeout end,
    spawn(?MODULE, send_after, [Self, 20, other]),
    C = receive late -> late after ?MODULE:id(110) -> timeout end,
    D = wait(0),
    spawn(?MODULE, send_after, [Self, 20, late]),
    E = receive late -> late after ?MODULE:id(1000) -> timeout end,
    F = wait(0),
    [A, B, C, D, E, F].

timeout_values() ->
    [receive_with(T)
     || T <- [1, -1, 4294967296, foo, [], {100}, <<"100">>, self()]].

receive_with(Time) ->
    try receive _ -> got after Time -> timeout end
    catch Class:Reason -> {Class, Reason}
    end.

receive_limits() ->
    Self = self(),
    Self ! other,
    spawn(?MODULE, send_after, [Self, 10, match]),
    A = receive match -> ok after ?MODULE:id(4294967295) -> timeout end,
    spawn(?MODULE, send_after, [Self, 10, match]),
    B = receive match -> ok after ?MODULE:id(infinity) -> timeout end,
    C = receive other -> other end,
    D = wait(0),
    [A, B, C, D].

start_timer() ->
    Self = self(),
    R1 = erlang:start_timer(?MODULE:id(60), Self, a),
    R2 = erlang:start_timer(?MODULE:id(30), Self, b),
    M1 = receive X -> X end,
    M2 = receive Y -> Y end,
    Bad = [start_bad(T, To)
           || {T, To} <- [{-1, Self}, {foo, Self}, {[], Self}, {10, "name"},
                          {10, 1}, {10, make_ref()}, {10, {timer_probe, node()}}]],
    [M1 =:= {timeout, R2, b}, M2 =:= {timeout, R1, a}, wait(0), Bad].

start_bad(Time, To) ->
    try erlang:start_timer(Time, To, x)
    catch Class:Reason -> {Class, Reason}
    end.

%% Time left of a 200 ms timer that is canceled at once.
left(L) when is_integer(L), L > 100, L =< 200 -> true;
left(L) -> L.

cancel_timer() ->
    Self = self(),
    R1 = erlang:start_timer(200, Self, x),
    A = left(erlang:cancel_timer(R1, [])),
    B = erlang:cancel_timer(R1, []),
    R2 = erlang:start_timer(200, Self, x),
    C = erlang:cancel_timer(R2, [{info, false}]),
    D = erlang:cancel_timer(R2, [{info, false}]),
    R3 = erlang:start_timer(200, Self, x),
    E = erlang:cancel_timer(R3, [{async, true}]),
    F = receive {cancel_timer, R3, L1} -> left(L1) end,
    G = erlang:cancel_timer(R3, [{async, true}, {info, true}]),
    H = receive {cancel_timer, R3, L2} -> L2 end,
    R4 = erlang:start_timer(200, Self, x),
    I = erlang:cancel_timer(R4, [{async, true}, {info, false}]),
    R5 = erlang:start_timer(0, Self, y),
    J = receive {timeout, R5, y} -> erlang:cancel_timer(R5, [{async, false}]) end,
    K = erlang:cancel_timer(make_ref(), [{async, true}, {async, false}]),
    L = erlang:cancel_timer(make_ref(), [{info, false}, {info, true}]),
    N = wait(300),
    Bad = [cancel_bad(T, O)
           || {T, O} <- [{foo, []}, {1, []}, {Self, []}, {R1, foo}, {R1, [foo]},
                         {R1, [{async, 1}]}, {R1, [{info, yes}]},
                         {R1, [{abs, true}]}, {R1, [{async, true, x}]},
                         {R1, [{other, true}]}, {foo, [{abs, false}]}]],
    [A, B, C, D, E, F, G, H, I, J, K, L, N, Bad].

%% erlang:cancel_timer/1 is cancel_timer/2 with [] (erl_hl_timer.c:2644-2647).
cancel_timer_1() ->
    R = erlang:start_timer(200, self(), x),
    A = left(erlang:cancel_timer(R)),
    B = erlang:cancel_timer(R),
    C = erlang:cancel_timer(make_ref()),
    D = try erlang:cancel_timer(foo) catch Class:Reason -> {Class, Reason} end,
    N = wait(300),
    [A, B, C, D, N].

cancel_bad(Ref, Options) ->
    try erlang:cancel_timer(Ref, Options)
    catch Class:Reason -> {Class, Reason}
    end.

receiver_exit() ->
    Self = self(),
    {Dead, Ref1} = spawn_monitor(?MODULE, id, [x]),
    receive {'DOWN', Ref1, process, Dead, normal} -> ok end,
    R1 = erlang:start_timer(100, Dead, x),
    A = erlang:cancel_timer(R1, []),
    {Live, Ref2} = spawn_monitor(?MODULE, wait, []),
    R2 = erlang:start_timer(1000, Live, x),
    Live ! stop,
    receive {'DOWN', Ref2, process, Live, normal} -> ok end,
    B = erlang:cancel_timer(R2, []),
    R3 = erlang:start_timer(30, timer_probe_name, named),
    true = register(timer_probe_name, Self),
    C = receive {timeout, R3, Msg} -> Msg after 1000 -> missing end,
    _ = erlang:start_timer(10, timer_probe_nobody, lost),
    D = wait(50),
    [A, B, C, D].

order() ->
    Self = self(),
    _ = [erlang:start_timer(T, Self, M)
         || {T, M} <- [{60, 60}, {50, 50}, {40, 40}, {20, 1}, {20, 2}, {20, 3}, {20, 4}]],
    Order = [receive {timeout, _, M} -> M end || _ <- [1, 2, 3, 4, 5, 6, 7]],
    R = erlang:start_timer(30, Self, bif),
    Recv = receive never -> never after ?MODULE:id(30) -> recv end,
    Bif = receive {timeout, R, bif} -> bif after 0 -> none end,
    [Order, Recv, Bif].

timer_limits() ->
    Self = self(),
    A = left_big(erlang:cancel_timer(erlang:start_timer(4294967296, Self, x), [])),
    B = left_big(erlang:cancel_timer(erlang:start_timer(9223372000000, Self, x), [])),
    C = start_bad(9223372036000, Self),
    D = start_bad(9223372036854775807, Self),
    [A, B, C, D].

left_big(L) when is_integer(L), L > 4294967200 -> ok;
left_big(L) -> L.

%% erlang:send_after/3 sends Msg, not {timeout, Ref, Msg}
%% (erl_hl_timer.c:2566-2581, 1824), with the checks, the reference, and the
%% timer of erlang:start_timer/3.
send_after_timer() ->
    Self = self(),
    R1 = erlang:send_after(?MODULE:id(60), Self, a),
    R2 = erlang:send_after(?MODULE:id(30), Self, b),
    M1 = receive X -> X end,
    M2 = receive Y -> Y end,
    A = left(erlang:cancel_timer(erlang:send_after(200, Self, c), [])),
    _ = erlang:send_after(30, timer_probe_after, named),
    true = register(timer_probe_after, Self),
    B = receive named -> named after 1000 -> missing end,
    true = unregister(timer_probe_after),
    Bad = [after_bad(T, To)
           || {T, To} <- [{-1, Self}, {foo, Self}, {10, "name"}, {10, 1},
                          {10, {timer_probe, node()}}]],
    [is_reference(R1), is_reference(R2), M1, M2, A, B, wait(0), Bad].

after_bad(Time, To) ->
    try erlang:send_after(Time, To, x)
    catch Class:Reason -> {Class, Reason}
    end.

%% erlang:start_timer/4 with the relative and absolute options
%% (erl_hl_timer.c:2622-2642).
start_timer_4() ->
    Self = self(),
    R1 = erlang:start_timer(?MODULE:id(60), Self, a, []),
    R2 = erlang:start_timer(?MODULE:id(30), Self, b, [{abs, false}]),
    M1 = receive X -> X end,
    M2 = receive Y -> Y end,
    Now = erlang:monotonic_time(millisecond),
    R3 = erlang:start_timer(Now, Self, c, [{abs, true}]),
    M3 = receive Z -> Z end,
    {Dead, MRef} = spawn_monitor(?MODULE, id, [x]),
    receive {'DOWN', MRef, process, Dead, normal} -> ok end,
    RD = erlang:start_timer(10, Dead, d, []),
    Bad = [start_4_bad(T, To, O)
           || {T, To, O} <- [{-1, Self, []}, {foo, Self, []}, {10, Self, foo},
                             {10, Self, [foo]}, {10, Self, [abs]},
                             {10, Self, [{abs, 1}]}, {10, Self, [{async, true}]},
                             {10, Self, [{info, true}]}, {10, 1, []},
                             {-576460753000, Self, [{abs, true}]},
                             {1 bsl 62, Self, [{abs, true}]}]],
    [M1 =:= {timeout, R2, b}, M2 =:= {timeout, R1, a}, M3 =:= {timeout, R3, c},
     is_reference(RD), wait(0), Bad].

start_4_bad(Time, To, Options) ->
    try erlang:start_timer(Time, To, x, Options)
    catch Class:Reason -> {Class, Reason}
    end.

%% erlang:send_after/4 with the relative and absolute options
%% (erl_hl_timer.c:2583-2603).
send_after_4() ->
    Self = self(),
    R1 = erlang:send_after(?MODULE:id(60), Self, a, []),
    R2 = erlang:send_after(?MODULE:id(30), Self, b, [{abs, false}]),
    M1 = receive X -> X end,
    M2 = receive Y -> Y end,
    Now = erlang:monotonic_time(millisecond),
    erlang:send_after(Now, Self, c, [{abs, true}]),
    M3 = receive Z -> Z end,
    Bad = [after_4_bad(T, To, O)
           || {T, To, O} <- [{-1, Self, []}, {foo, Self, []}, {10, Self, [foo]},
                             {10, Self, [{abs, 1}]}, {10, Self, [{async, true}]},
                             {10, 1, []}]],
    [is_reference(R1), is_reference(R2), M1 =:= b, M2 =:= a, M3 =:= c,
     wait(0), Bad].

after_4_bad(Time, To, Options) ->
    try erlang:send_after(Time, To, x, Options)
    catch Class:Reason -> {Class, Reason}
    end.

%% erlang:read_timer/1 on a live, cancelled, and expired timer
%% (erl_hl_timer.c:2661-2664).
read_timer() ->
    Self = self(),
    R1 = erlang:start_timer(?MODULE:id(200), Self, x),
    A = left(erlang:read_timer(R1)),
    B = left(erlang:read_timer(R1)),
    C = left(erlang:cancel_timer(R1)),
    D = erlang:read_timer(R1),
    R2 = erlang:start_timer(0, Self, y),
    _ = receive {timeout, R2, y} -> ok end,
    E = erlang:read_timer(R2),
    F = erlang:read_timer(make_ref()),
    G = try erlang:read_timer(foo) catch Class:Reason -> {Class, Reason} end,
    [A, B, C, D, E, F, G, wait(0)].
