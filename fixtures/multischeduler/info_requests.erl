-module(info_requests).
-export([leader_a/0, leader_b/0, alive_a/0, alive_b/0, info_a/0, info_b/0,
         reference/0]).

-define(ITEMS, [registered_name, message_queue_len, group_leader, priority,
                links]).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun leader_a/0, fun leader_b/0),
     pair(fun alive_a/0, fun alive_b/0),
     pair(fun info_a/0, fun info_b/0)].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% Each root returns ok when its result is the expected result, and else
%% {unexpected, Result}.
leader_a() -> check(leader_a_run(), {true, true, badarg, undefined}).
leader_b() -> check(leader_b_run(), {true, true}).
alive_a() -> check(alive_a_run(), {true, false, false}).
alive_b() -> check(alive_b_run(), killed).
info_a() ->
    Holder = wait_for(info_b),
    Live = [{registered_name, info_child}, {message_queue_len, 3},
            {group_leader, Holder}, {priority, high}, {links, [Holder]}, true],
    check(info_a_run(), {Live, lists:duplicate(6, undefined)}).
info_b() -> check(info_b_run(), normal).

check(Expected, Expected) -> ok;
check(Result, _) -> {unexpected, Result}.

wait_for(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name) end;
        Pid -> Pid
    end.

%% The root that reads the group leader of a process of the other scheduler,
%% makes itself that group leader, and reads it again. After the exit of the
%% process, the group-leader request raises badarg and the read gives
%% undefined.
leader_a_run() ->
    Holder = wait_for(leader_b),
    {group_leader, Initial} = process_info(Holder, group_leader),
    true = group_leader(self(), Holder),
    Changed = process_info(Holder, group_leader) =:= {group_leader, self()},
    Ref = monitor(process, Holder),
    Holder ! {stop, self(), Initial},
    receive {'DOWN', Ref, process, Holder, normal} -> ok end,
    Dead = try group_leader(self(), Holder) catch error:badarg -> badarg end,
    {is_pid(Initial), Changed, Dead, process_info(Holder, group_leader)}.

%% The root that holds a name. It checks that the other root read its first
%% group leader and that the change reached it.
leader_b_run() ->
    Initial = group_leader(),
    true = register(leader_b, self()),
    receive
        {stop, Leader, Seen} -> {Seen =:= Initial, group_leader() =:= Leader}
    end.

%% The root that checks the liveness of a process of the other scheduler
%% before and after its exit signal kill, and after its DOWN.
alive_a_run() ->
    Child = wait_for(alive_child),
    Ref = monitor(process, Child),
    Live = is_process_alive(Child),
    true = exit(Child, kill),
    Killed = is_process_alive(Child),
    receive {'DOWN', Ref, process, Child, killed} -> ok end,
    {Live, Killed, is_process_alive(Child)}.

%% The root whose child the other root kills. It returns the exit reason of
%% the child.
alive_b_run() ->
    {Child, Ref} = spawn_monitor(fun alive_child/0),
    receive {'DOWN', Ref, process, Child, Reason} -> Reason end.

alive_child() ->
    true = register(alive_child, self()),
    receive stop -> ok end.

%% The root that sends three messages to a process of the other scheduler,
%% reads its items, and reads them again after its exit.
info_a_run() ->
    Child = wait_for(info_child),
    _ = [Child ! {m, I} || I <- [1, 2, 3]],
    Live = [process_info(Child, Item) || Item <- ?ITEMS],
    {reductions, Reductions} = process_info(Child, reductions),
    Ref = monitor(process, Child),
    Child ! stop,
    receive {'DOWN', Ref, process, Child, normal} -> ok end,
    Dead = [process_info(Child, Item) || Item <- ?ITEMS ++ [reductions]],
    {Live ++ [is_integer(Reductions)], Dead}.

%% The root that holds a name, starts a linked child with the priority high,
%% and makes itself the group leader of the child. It returns the exit
%% reason of the child.
info_b_run() ->
    true = register(info_b, self()),
    Child = spawn_link(fun info_child/0),
    Ref = monitor(process, Child),
    true = group_leader(self(), Child),
    Child ! go,
    receive {'DOWN', Ref, process, Child, Reason} -> Reason end.

info_child() ->
    _ = process_flag(priority, high),
    receive go -> ok end,
    true = register(info_child, self()),
    receive stop -> ok end.
