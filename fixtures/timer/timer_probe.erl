-module(timer_probe).
-export([receive_after/0, timeout_values/0, receive_limits/0, send_after/3, id/1]).

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant timeout or argument.
id(X) -> X.

%% Send Msg to Pid after Time milliseconds, with a receive timeout.
send_after(Pid, Time, Msg) ->
    receive after Time -> Pid ! Msg end.

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
