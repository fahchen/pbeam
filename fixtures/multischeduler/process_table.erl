-module(process_table).
-export([list_a/0, list_b/0, gone_a/0, gone_b/0, hold/0, reference/0]).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun list_a/0, fun list_b/0),
     pair(fun gone_a/0, fun gone_b/0)].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% Each root returns ok when its result is the expected result, and else
%% {unexpected, Result}.
list_a() -> check(list_a_run(), {true, true, true}).
list_b() -> check(list_b_run(), {true, true}).
gone_a() -> check(gone_a_run(), {killed, false, false, false}).
gone_b() -> check(gone_b_run(), {false, false}).

check(Expected, Expected) -> ok;
check(Result, _) -> {unexpected, Result}.

wait_for(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name) end;
        Pid -> Pid
    end.

hold() ->
    receive stop -> ok end.

%% Return the pids of erlang:processes_iterator/0 and
%% erlang:processes_next/1.
iterate(Iterator, Acc) ->
    case erlang:processes_next(Iterator) of
        none -> lists:reverse(Acc);
        {Pid, Next} -> iterate(Next, [Pid | Acc])
    end.

%% The root that reads the process list after each root started a child.
%% Its list has both roots and both children, the other root reads the
%% same list, and the process iterator gives the same pids in the same
%% order.
list_a_run() ->
    register(list_a, self()),
    Child = spawn(?MODULE, hold, []),
    Other = wait_for(list_b),
    OtherChild = receive {child, Other, C} -> C end,
    Mine = processes(),
    Iterated = iterate(erlang:processes_iterator(), []),
    Other ! {list, self(), Mine},
    Same = receive {same, Other, S} -> S end,
    Child ! stop,
    Members = lists:all(fun(Pid) -> lists:member(Pid, Mine) end,
                        [self(), Child, Other, OtherChild]),
    {Members, Same, Iterated =:= Mine}.

%% The root that starts a child, then reads the process list of the other
%% root and its own list.
list_b_run() ->
    register(list_b, self()),
    Child = spawn(?MODULE, hold, []),
    Other = wait_for(list_a),
    Other ! {child, self(), Child},
    Theirs = receive {list, Other, L} -> L end,
    Mine = processes(),
    Other ! {same, self(), Mine =:= Theirs},
    Child ! stop,
    {lists:member(Child, Mine), Mine =:= Theirs}.

%% The root that waits for the exit of a child of the other root. After
%% the DOWN message, the child is not in the process list, the process
%% iterator does not give it, and it is not alive. The monitor signal
%% reaches the child before the kill of the other root, so the DOWN reason
%% is killed.
gone_a_run() ->
    register(gone_a, self()),
    Other = wait_for(gone_b),
    Child = receive {child, Other, C} -> C end,
    Ref = monitor(process, Child),
    Other ! {watching, self()},
    Reason = receive {'DOWN', Ref, process, Child, R} -> R end,
    Other ! {down, self()},
    {Reason,
     lists:member(Child, processes()),
     lists:member(Child, iterate(erlang:processes_iterator(), [])),
     is_process_alive(Child)}.

%% The root that kills its child after the other root monitors it, and
%% reads its own process list after the DOWN message of the other root.
gone_b_run() ->
    register(gone_b, self()),
    Child = spawn(?MODULE, hold, []),
    Other = wait_for(gone_a),
    Other ! {child, self(), Child},
    receive {watching, Other} -> ok end,
    exit(Child, kill),
    receive {down, Other} -> ok end,
    {lists:member(Child, processes()), is_process_alive(Child)}.
