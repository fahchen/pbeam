-module(monitor_signals).
-export([order_watch/0, order_owner/0, dead_watch/0, dead_owner/0,
         flush_watch/0, flush_owner/0, race_watch/0, race_owner/0,
         kill_watch/0, kill_owner/0, slot_watch/0, slot_owner/0,
         reference/0]).

-define(RACES, 20).
-define(KILLS, 200).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun order_watch/0, fun order_owner/0),
     pair(fun dead_watch/0, fun dead_owner/0),
     pair(fun flush_watch/0, fun flush_owner/0),
     pair(fun race_watch/0, fun race_owner/0),
     pair(fun kill_watch/0, fun kill_owner/0),
     pair(fun slot_watch/0, fun slot_owner/0)].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% Each root returns ok when its result is the expected result, and else
%% {unexpected, Result}.
order_watch() -> check(order_watch_run(), order_log()).
order_owner() -> check(owner_run(order_owner), ok).
dead_watch() -> check(dead_watch_run(), dead_log()).
dead_owner() -> check(owner_run(dead_owner), ok).
flush_watch() -> check(flush_watch_run(), flush_log()).
flush_owner() -> check(owner_run(flush_owner), ok).
race_watch() -> check(race_watch_run(), race_log()).
race_owner() -> check(owner_run(race_owner), ok).
kill_watch() -> check(kill_watch_run() -- [killed, noproc], []).
kill_owner() -> check(owner_run(kill_owner), ok).
slot_watch() -> check(slot_watch_run(),
                             [one, three, three, six, five, noproc, noproc, n, n]).
slot_owner() -> check(owner_run(slot_owner), ok).

check(Expected, Expected) -> ok;
check(Result, _) -> {unexpected, Result}.

wait_for(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name) end;
        Pid -> Pid
    end.

%% The root of the other scheduler. It spawns the children that the watcher
%% asks for, gives a name to a child on request, and kills a list of
%% children on request, until the watcher is done.
owner_run(Name) ->
    true = register(Name, self()),
    owner_serve().

owner_serve() ->
    receive
        {spawn, Watcher} ->
            Watcher ! {child, spawn(fun child/0)},
            owner_serve();
        {named, Watcher} ->
            Child = spawn(fun child/0),
            true = register(monitor_signals_named, Child),
            Watcher ! {child, Child},
            owner_serve();
        {kill, Watcher, Children} ->
            [true = exit(Child, kill) || Child <- Children],
            Watcher ! killed,
            owner_serve();
        done ->
            ok
    end.

%% A child that sends `Count` messages to `To` and then exits with `Reason`
%% when it gets `{go, To, Count, Reason}`.
child() ->
    receive
        {go, To, Count, Reason} ->
            [To ! {n, I} || I <- lists:seq(1, Count)],
            exit(Reason)
    end.

remote_child(Owner) ->
    Owner ! {spawn, self()},
    receive {child, Child} -> Child end.

%% Collect each message until `Downs` DOWN messages arrived, in arrival
%% order, and then each message that arrives in 20 milliseconds.
collect(0, Log) ->
    lists:reverse(Log) ++ rest();
collect(Downs, Log) ->
    receive
        {_, _, process, _, _} = Down -> collect(Downs - 1, [down(Down) | Log]);
        Message -> collect(Downs, [Message | Log])
    end.

rest() ->
    receive Message -> [{late, Message}] ++ rest() after 20 -> [] end.

down({Tag, _, process, _, Reason}) -> {Tag, Reason}.

%% The messages of the target come in send order before its DOWN messages,
%% each monitor gives one DOWN, and a monitor that ended before the exit
%% gives none. The last monitor ends while its monitor signal still waits to
%% be sent, so its demonitor signal follows it.
order_log() ->
    [{n, I} || I <- lists:seq(1, 20)] ++ [{'DOWN', boom}, {gone, boom}].

order_watch_run() ->
    Owner = wait_for(order_owner),
    Child = remote_child(Owner),
    Ended = monitor(process, Child),
    Ref = monitor(process, Child),
    true = demonitor(Ended),
    Tagged = monitor(process, Child, [{tag, gone}]),
    Pending = monitor(process, Child),
    true = demonitor(Pending),
    Child ! {go, self(), 20, boom},
    Log = collect(2, []),
    Owner ! done,
    true = is_reference(Ref) andalso is_reference(Tagged),
    Log.

%% A monitor of a remote process that exited gives one DOWN with `noproc`.
%% A monitor by the name of a remote process gives the name in its DOWN.
dead_log() ->
    {[{'DOWN', noproc}], true, false,
     [{'DOWN', {monitor_signals_named, node()}, normal}],
     [{'DOWN', {monitor_signals_named, node()}, noproc}]}.

dead_watch_run() ->
    Owner = wait_for(dead_owner),
    Child = remote_child(Owner),
    First = monitor(process, Child),
    Child ! {go, self(), 0, normal},
    receive {'DOWN', First, process, Child, normal} -> ok end,
    Ref = monitor(process, Child),
    Dead = collect(1, []),
    Plain = demonitor(Ref),
    Info = demonitor(Ref, [info]),
    Owner ! {named, self()},
    Named = receive {child, Pid} -> Pid end,
    NamedRef = monitor(process, monitor_signals_named),
    Named ! {go, self(), 0, normal},
    NamedLog = named_down(NamedRef),
    GoneRef = monitor(process, monitor_signals_named),
    GoneLog = named_down(GoneRef),
    Owner ! done,
    {Dead, Plain, Info, NamedLog, GoneLog}.

named_down(Ref) ->
    receive
        {'DOWN', Ref, process, Item, Reason} -> [{'DOWN', Item, Reason}] ++ rest()
    end.

%% `demonitor/2` with `flush` removes the DOWN message of its monitor only,
%% and a demonitor that races with the exit leaves no DOWN message.
flush_log() ->
    {true, false, [{'DOWN', 1}, {'DOWN', 3}, {other, 2}], [none]}.

flush_watch_run() ->
    Owner = wait_for(flush_owner),
    Children = [remote_child(Owner) || _ <- [1, 2, 3]],
    [R1, R2, R3] = [monitor(process, Child) || Child <- Children],
    Owner ! {kill, self(), Children},
    receive killed -> ok end,
    wait_queue(3),
    self() ! {other, R2},
    Flush = demonitor(R2, [flush]),
    Info = demonitor(R1, [flush, info]),
    self() ! {'DOWN', R1, process, self(), fake},
    Left = lists:sort([label(Message, [R1, R2, R3]) || Message <- take_all()]),
    Races = lists:usort([flush_race(Owner) || _ <- lists:seq(1, ?RACES)]),
    Owner ! done,
    {Flush, Info, Left, Races}.

wait_queue(Length) ->
    case process_info(self(), message_queue_len) of
        {message_queue_len, Length} -> ok;
        _ -> receive after 1 -> wait_queue(Length) end
    end.

label({'DOWN', Ref, process, _, _}, Refs) -> {'DOWN', index(Ref, Refs, 1)};
label({other, Ref}, Refs) -> {other, index(Ref, Refs, 1)}.

index(Ref, [Ref | _], I) -> I;
index(Ref, [_ | Refs], I) -> index(Ref, Refs, I + 1).

take_all() ->
    receive Message -> [Message | take_all()] after 0 -> [] end.

flush_race(Owner) ->
    Child = remote_child(Owner),
    Ref = monitor(process, Child),
    Child ! {go, self(), 0, boom},
    true = demonitor(Ref, [flush]),
    receive {'DOWN', Ref, _, _, _} = Down -> Down after 20 -> none end.

%% A monitor that races with the exit of a remote process gives one DOWN with
%% its exit reason or `noproc`.
race_log() ->
    [true].

race_watch_run() ->
    Owner = wait_for(race_owner),
    Results = lists:usort([monitor_race(Owner) || _ <- lists:seq(1, ?RACES)]),
    Owner ! done,
    Results.

monitor_race(Owner) ->
    Child = remote_child(Owner),
    Child ! {go, self(), 0, boom},
    Ref = monitor(process, Child),
    receive
        {'DOWN', Ref, process, Child, Reason} ->
            lists:member(Reason, [boom, noproc]) andalso rest() =:= []
    end.

%% Each monitor gives one DOWN with the reason `killed` or `noproc`. ERTS
%% keeps the order of signals of one sender only
%% (system/doc/reference_manual/ref_man_processes.md:507-511), so the kill
%% from the owner can reach the child before the monitor signal of the
%% watcher. The pinned ERTS gives `noproc` in about 1 of 100 runs. The child
%% is a process of the owner, or a process of the watcher.
kill_watch_run() ->
    Owner = wait_for(kill_owner),
    Remote = [monitor_kill(Owner, remote_child(Owner)) || _ <- lists:seq(1, ?KILLS)],
    Local = [monitor_kill(Owner, spawn(fun child/0)) || _ <- lists:seq(1, ?KILLS)],
    Owner ! done,
    lists:usort(Remote ++ Local).

monitor_kill(Owner, Child) ->
    Ref = monitor(process, Child),
    Owner ! {kill, self(), [Child]},
    receive killed -> ok end,
    receive {'DOWN', Ref, process, Child, Reason} -> Reason end.

%% Each monitor signal waits in the pending slot while the watcher sends to
%% other remote children. A message to the target of the slot sends the
%% monitor signal first, a second monitor signal replaces it in the slot,
%% the switch out sends it, and a link to a dead target sends it. Each held
%% message and exit signal arrives, and each monitor reaches its child
%% before the child exits.
slot_watch_run() ->
    Owner = wait_for(slot_owner),
    [C1, C2, C3, C4, C5, C6, C7] = [remote_child(Owner) || _ <- lists:seq(1, 7)],
    R1 = monitor(process, C1),
    C2 ! {go, self(), 0, two},
    C1 ! {go, self(), 0, one},
    R3 = monitor(process, C3),
    C4 ! {go, self(), 1, four},
    R4 = monitor(process, C3),
    C3 ! {go, self(), 0, three},
    R6 = monitor(process, C6),
    R5 = monitor(process, C5),
    true = exit(C6, six),
    Six = receive {'DOWN', R6, process, _, Reason6} -> Reason6 end,
    C5 ! {go, self(), 0, five},
    process_flag(trap_exit, true),
    {Dead, RD} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', RD, process, _, _} -> ok end,
    R7 = monitor(process, Dead),
    C7 ! {go, self(), 1, seven},
    true = link(Dead),
    Owner ! done,
    Ns = [receive {n, 1} -> n after 5000 -> timeout end || _ <- [C4, C7]],
    Exit = receive {'EXIT', Dead, ExitReason} -> ExitReason end,
    [receive {'DOWN', R, process, _, Reason} -> Reason end
     || R <- [R1, R3, R4]] ++ [Six]
        ++ [receive {'DOWN', R, process, _, Reason} -> Reason end
            || R <- [R5, R7]] ++ [Exit | Ns].
