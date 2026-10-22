-module(link_signals).
-export([order_send/0, order_recv/0, dead_a/0, dead_b/0, race_a/0, race_b/0,
         exit_a/0, exit_b/0, reference/0]).

-define(RACES, 20).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun order_send/0, fun order_recv/0),
     pair(fun dead_a/0, fun dead_b/0),
     pair(fun race_a/0, fun race_b/0),
     pair(fun exit_a/0, fun exit_b/0)].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% Each root returns ok when its result is the expected result, and else
%% {unexpected, Result}.
order_send() -> check(order_send_run(), boom).
order_recv() -> check(order_recv_run(), order_log()).
dead_a() -> check(dead_a_run(), {noproc, true, noproc, true, true, none}).
dead_b() -> check(dead_b_run(), ok).
race_a() -> check(race_a_run(), [{kill, true}, {late_link, true}, {linked, boom}, {unlink, none}]).
race_b() -> check(race_b_run(), ok).
exit_a() -> check(exit_a_run(), {pong, killed, x}).
exit_b() -> check(exit_b_run(), ok).

check(Expected, Expected) -> ok;
check(Result, _) -> {unexpected, Result}.

%% The log of the receiver: each message and exit signal of the child in send
%% order, with the link state at each acknowledged message.
order_log() ->
    Burst = lists:append([[{n, I}, {exit, {e, I}}] || I <- lists:seq(1, 20)]),
    [{a, true}] ++ Burst ++ [{b, false}, {c, true}, {{exit, boom}, false}].

wait_for(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name) end;
        Pid -> Pid
    end.

%% The root whose child sends messages, link, unlink, and exit signals to the
%% receiver of the other scheduler, and then exits. Return the exit reason
%% of the child.
order_send_run() ->
    Recv = wait_for(order_recv),
    {Child, Ref} = spawn_monitor(fun() -> order_child(Recv) end),
    receive {'DOWN', Ref, process, Child, Reason} -> Reason end.

order_child(Recv) ->
    Recv ! {child, self()},
    true = link(Recv),
    Recv ! a,
    receive ack -> ok end,
    [begin Recv ! {n, I}, true = exit(Recv, {e, I}) end || I <- lists:seq(1, 20)],
    true = unlink(Recv),
    Recv ! b,
    receive ack -> ok end,
    true = link(Recv),
    Recv ! c,
    receive ack -> ok end,
    exit(boom).

%% The receiver that traps exits. It records each message in arrival order.
%% At each message that the child waits for, it records whether the link to
%% the child is active and acknowledges the message.
order_recv_run() ->
    process_flag(trap_exit, true),
    true = register(order_recv, self()),
    Child = receive {child, Pid} -> Pid end,
    order_collect(Child, []).

order_collect(Child, Log) ->
    receive
        {'EXIT', Child, boom} ->
            lists:reverse([{{exit, boom}, linked(Child)} | Log]);
        {'EXIT', Child, Reason} ->
            order_collect(Child, [{exit, Reason} | Log]);
        {n, _} = Message ->
            order_collect(Child, [Message | Log]);
        Message ->
            Entry = {Message, linked(Child)},
            Child ! ack,
            order_collect(Child, [Entry | Log])
    end.

linked(Pid) ->
    {links, Links} = process_info(self(), links),
    lists:member(Pid, Links).

%% The root that sends link, unlink, and exit signals to a process of the
%% other scheduler that exited.
dead_a_run() ->
    B = wait_for(dead_b),
    B ! {a, self()},
    Child = receive {child, Pid} -> Pid end,
    Ref = monitor(process, Child),
    receive {'DOWN', Ref, process, Child, _} -> ok end,
    NoTrap = try link(Child) catch error:Reason -> Reason end,
    process_flag(trap_exit, true),
    Trap = link(Child),
    Exit = receive {'EXIT', Child, ExitReason} -> ExitReason after 1000 -> none end,
    Unlink = unlink(Child),
    Kill = exit(Child, kill),
    Rest = receive Message -> Message after 10 -> none end,
    B ! done,
    {NoTrap, Trap, Exit, Unlink, Kill, Rest}.

dead_b_run() ->
    true = register(dead_b, self()),
    A = receive {a, Pid} -> Pid end,
    A ! {child, spawn(fun() -> ok end)},
    receive done -> ok end.

%% The root that races its sends and signals with the exit of processes of
%% the other scheduler. Each check holds for each order that the two threads
%% can take.
race_a_run() ->
    B = wait_for(race_b),
    process_flag(trap_exit, true),
    Results = [race(I rem 4, B) || I <- lists:seq(1, 4 * ?RACES)],
    B ! done,
    lists:usort(Results).

race(Kind, B) ->
    B ! {spawn, self()},
    Child = receive {child, Pid} -> Pid end,
    race_kind(Kind, Child).

%% A link before the exit gives the exit reason, and each send to the
%% exiting process returns its message.
race_kind(0, Child) ->
    true = link(Child),
    Child ! go,
    [{n, I} = Child ! {n, I} || I <- lists:seq(1, 100)],
    receive {'EXIT', Child, Reason} -> {linked, Reason} end;
%% A link that races with the exit gives the exit reason or noproc.
race_kind(1, Child) ->
    Child ! go,
    true = link(Child),
    receive
        {'EXIT', Child, Reason} -> {late_link, lists:member(Reason, [boom, noproc])}
    end;
%% An unlink that races with the exit removes the link, so the exit signal of
%% the child gives no message. The child sends its exit signal before its
%% monitor down signal.
race_kind(2, Child) ->
    true = link(Child),
    Ref = monitor(process, Child),
    Child ! go,
    true = unlink(Child),
    receive {'DOWN', Ref, process, Child, boom} -> ok end,
    receive {'EXIT', Child, _} = Exit -> {unlink, Exit} after 0 -> {unlink, none} end;
%% An exit signal that races with the exit gives one of the two reasons.
race_kind(3, Child) ->
    Ref = monitor(process, Child),
    Child ! go,
    true = exit(Child, kill),
    receive
        {'DOWN', Ref, process, Child, Reason} -> {kill, lists:member(Reason, [boom, killed])}
    end.

race_b_run() ->
    true = register(race_b, self()),
    race_serve().

race_serve() ->
    receive
        {spawn, A} ->
            A ! {child, spawn(fun() -> receive go -> exit(boom) end end)},
            race_serve();
        done ->
            ok
    end.

%% The root that sends exit signals to a process of the other scheduler that
%% does not trap exits: `normal` is ignored, and `kill` ends it. Then it links
%% to a second process before a message that makes that process exit, so it
%% gets the exit reason and not `noproc`.
exit_a_run() ->
    B = wait_for(exit_b),
    B ! {a, self()},
    {Child, Dier} = receive {children, Pid, Other} -> {Pid, Other} end,
    Ref = monitor(process, Child),
    true = exit(Child, normal),
    Child ! {ping, self()},
    Pong = receive pong -> pong end,
    true = exit(Child, kill),
    Reason = receive {'DOWN', Ref, process, Child, Down} -> Down end,
    process_flag(trap_exit, true),
    true = link(Dier),
    Dier ! die,
    Linked = receive {'EXIT', Dier, Exit} -> Exit end,
    B ! done,
    {Pong, Reason, Linked}.

exit_b_run() ->
    true = register(exit_b, self()),
    A = receive {a, Pid} -> Pid end,
    A ! {children, spawn(fun exit_loop/0), spawn(fun() -> receive die -> exit(x) end end)},
    receive done -> ok end.

exit_loop() ->
    receive {ping, From} -> From ! pong, exit_loop() end.
