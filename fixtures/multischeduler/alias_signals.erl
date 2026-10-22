-module(alias_signals).
-export([call_client/0, call_server/0, alias_client/0, alias_owner/0,
         monitor_client/0, monitor_server/0, reference/0]).

-define(CALLS, 50).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun call_client/0, fun call_server/0),
     pair(fun alias_client/0, fun alias_owner/0),
     pair(fun monitor_client/0, fun monitor_server/0)].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% Each root returns ok when its result is the expected result, and else
%% {unexpected, Result}.
call_client() -> check(call_client_run(), lists:seq(1, ?CALLS)).
call_server() -> check(call_server_run(), ok).
alias_client() -> check(alias_client_run(), ok).
alias_owner() -> check(alias_owner_run(), [one, two, three]).
monitor_client() -> check(monitor_client_run(), [{reply, ok}, normal, late, []]).
monitor_server() -> check(monitor_server_run(), ok).

check(Expected, Expected) -> ok;
check(Result, _) -> {unexpected, Result}.

wait_for(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name) end;
        Pid -> Pid
    end.

take_all() ->
    receive Message -> [Message | take_all()] after 0 -> [] end.

%% gen:call/3 to a process of the other scheduler: the request carries an
%% alias of a monitor with the `alias` option, and the reply goes to it.
call_client_run() ->
    Server = wait_for(alias_signals_server),
    Replies = [begin {ok, Reply} = gen:call(Server, '$gen_call', N), Reply end
               || N <- lists:seq(1, ?CALLS)],
    Server ! done,
    Replies.

call_server_run() ->
    true = register(alias_signals_server, self()),
    call_serve().

call_serve() ->
    receive
        {'$gen_call', From, N} ->
            ok = gen:reply(From, N),
            call_serve();
        done ->
            ok
    end.

%% The client sends to the aliases of a process of the other scheduler. A
%% `reply` alias takes one message, an `explicit_unalias` alias takes each
%% message until `unalias/1`, and a send after it drops the message. A send
%% to an active alias after the exit of its owner drops the message.
alias_client_run() ->
    Owner = wait_for(alias_signals_owner),
    Owner ! {client, self()},
    receive {aliases, Reply, Explicit, Kept} -> ok end,
    Monitor = monitor(process, Owner),
    Reply ! one,
    Reply ! dropped,
    Explicit ! two,
    Explicit ! three,
    Owner ! sync,
    receive unaliased -> ok end,
    Explicit ! dropped,
    Owner ! done,
    receive {'DOWN', Monitor, process, Owner, normal} -> ok end,
    Kept ! dropped,
    ok.

alias_owner_run() ->
    true = register(alias_signals_owner, self()),
    receive {client, Client} -> ok end,
    Reply = alias([reply]),
    Explicit = alias(),
    Kept = alias(),
    Client ! {aliases, Reply, Explicit, Kept},
    receive sync -> ok end,
    true = unalias(Explicit),
    false = unalias(Explicit),
    false = unalias(Reply),
    Client ! unaliased,
    receive done -> ok end,
    take_all().

%% A monitor of a process of the other scheduler with an
%% `explicit_unalias` alias: the reply goes to the alias, the alias stays
%% after the `DOWN`, and `unalias/1` ends it.
monitor_client_run() ->
    Server = wait_for(alias_signals_monitor),
    Ref = monitor(process, Server, [{alias, explicit_unalias}]),
    Server ! {request, Ref},
    Reply = receive {Ref, R} -> R end,
    Down = receive {'DOWN', Ref, process, Server, Reason} -> Reason end,
    Late = whereis(alias_signals_late),
    Late ! {again, Ref},
    Again = receive {Ref, A} -> A end,
    true = unalias(Ref),
    Late ! {again, Ref},
    Late ! {stop, self()},
    receive stopped -> ok end,
    [{reply, Reply}, Down, Again, take_all()].

monitor_server_run() ->
    Late = spawn(fun late/0),
    true = register(alias_signals_late, Late),
    true = register(alias_signals_monitor, self()),
    receive {request, Alias} -> Alias ! {Alias, ok} end,
    ok.

late() ->
    receive
        {again, Alias} -> Alias ! {Alias, late}, late();
        {stop, Client} -> Client ! stopped
    end.
