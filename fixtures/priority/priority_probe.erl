-module(priority_probe).
-export([orders/0, lows/0, wakes/0, exits/0, flags/0, spawns/0, worker/4, sleeper/1,
         exiter/2, ticker/1]).

%% Each worker sets its priority in its first turn. Then it sends its tag
%% to the parent and yields, N times. With one scheduler, the arrival
%% order of the tags is the run order of the workers. The parent waits in
%% a receive, so it does not run while a worker runs.

%% Workers of each priority, spawned in a mixed order.
orders() ->
    race([{l1, low}, {n1, normal}, {h1, high}, {m1, max}, {l2, low}, {n2, normal},
          {h2, high}, {m2, max}], 6).

%% Low workers with normal workers: a low process at the head of the
%% normal queue goes to the end until it was at the head 8 times, except
%% when it is the last process of the queue.
lows() ->
    [race([{l, low}, {n1, normal}, {n2, normal}, {n3, normal}], 12),
     race([{l, low}, {n, normal}], 12),
     race([{l1, low}, {l2, low}, {n, normal}], 12),
     race([{l, low}], 3)].

%% The parent monitors a process 5000 more times, and the process exits.
%% Each 'DOWN' message uses 1 reduction of the exit after the process
%% leaves the process table, so the exit switches out once and continues
%% in its next turn at its own priority. The number of ticks of a normal
%% ticker before the last 'DOWN' message shows where the exit continues.
exits() -> [{Priority, exit_ticks(Priority)} || Priority <- [low, normal, max]].

exit_ticks(Priority) ->
    Parent = self(),
    {Exiter, _} = spawn_monitor(?MODULE, exiter, [Parent, Priority]),
    receive {ready, Exiter} -> ok end,
    monitors(Exiter, 5000),
    Ticker = spawn(?MODULE, ticker, [Parent]),
    Exiter ! go,
    Ticks = ticks(5001, 0),
    exit(Ticker, kill),
    flush_ticks(),
    Ticks.

exiter(Parent, Priority) ->
    process_flag(priority, Priority),
    Parent ! {ready, self()},
    receive go -> ok end.

monitors(_, 0) -> ok;
monitors(Pid, N) -> _ = erlang:monitor(process, Pid), monitors(Pid, N - 1).

ticker(Parent) -> Parent ! tick, erlang:yield(), ticker(Parent).

ticks(0, N) -> N;
ticks(Downs, N) ->
    receive
        tick -> ticks(Downs, N + 1);
        {'DOWN', _, process, _, normal} -> ticks(Downs - 1, N)
    end.

flush_ticks() -> receive tick -> flush_ticks() after 0 -> ok end.

%% A message to a waiting max process puts it in the max queue. The
%% sender continues its turn, and the max process runs at the next switch.
wakes() ->
    Parent = self(),
    Max = spawn(?MODULE, sleeper, [Parent]),
    receive {ready, Max} -> ok end,
    High = spawn(fun() -> process_flag(priority, high), Max ! wake, Parent ! high, erlang:yield(), Parent ! high_after, Parent ! {done, self()} end),
    Normal = spawn(fun() -> Max ! wake, Parent ! normal, erlang:yield(), Parent ! normal_after, Parent ! {done, self()} end),
    collect([Max, High, Normal], []).

sleeper(Parent) ->
    process_flag(priority, max),
    Parent ! {ready, self()},
    sleep(Parent, 2).

sleep(Parent, 0) -> Parent ! {done, self()};
sleep(Parent, N) ->
    receive wake -> Parent ! max end,
    sleep(Parent, N - 1).

%% The results of process_flag/2 and process_info/2 for priorities.
flags() ->
    Parent = self(),
    Child = spawn(fun() -> process_flag(priority, low), Parent ! set, receive stop -> ok end end),
    receive set -> ok end,
    Values = [process_flag(priority, high), process_flag(priority, high),
              process_flag(priority, low), process_flag(priority, max),
              process_info(self(), priority), process_flag(priority, normal),
              error_of(priority, foo), error_of(priority, 1), error_of(priority, "max"),
              process_info(Child, priority)],
    Child ! stop,
    Values.

%% The spawn option {priority, Level} sets the priority of the new
%% process, which otherwise starts with normal. A max process does not
%% give its priority to its children.
spawns() ->
    Parent = self(),
    Child = fun() -> Parent ! {self(), process_info(self(), priority)} end,
    Pids = [spawn_opt(erlang, apply, [Child, []], Options)
            || Options <- [[], [{priority, low}], [{priority, high}, {priority, max}]]],
    Infos = [receive {Pid, Info} -> Info end || Pid <- Pids],
    process_flag(priority, max),
    Inherit = spawn(Child),
    Inherited = receive {Inherit, Info} -> Info end,
    process_flag(priority, normal),
    Bad = [try spawn_opt(erlang, apply, [Child, []], [{priority, Level}])
           catch Class:Reason -> {Class, Reason}
           end
           || Level <- [foo, 1]],
    {Infos, Inherited, Bad}.

error_of(Flag, Value) ->
    try process_flag(Flag, Value)
    catch Class:Reason -> {Class, Reason}
    end.

race(Workers, N) ->
    Parent = self(),
    Pids = [spawn(?MODULE, worker, [Parent, Tag, Priority, N]) || {Tag, Priority} <- Workers],
    runs(collect(Pids, [])).

worker(Parent, Tag, Priority, N) ->
    normal = process_flag(priority, Priority),
    loop(Parent, Tag, N).

loop(Parent, _Tag, 0) -> Parent ! {done, self()};
loop(Parent, Tag, N) ->
    Parent ! Tag,
    erlang:yield(),
    loop(Parent, Tag, N - 1).

%% Collect the tags until each process of Pids sent done. The probe calls
%% no other module, so PBeam loads only this module and erlang.
collect([], Tags) -> reverse(Tags, []);
collect(Pids, Tags) ->
    receive
        {done, Pid} -> collect([P || P <- Pids, P =/= Pid], Tags);
        Tag when is_atom(Tag) -> collect(Pids, [Tag | Tags])
    end.

reverse([], Acc) -> Acc;
reverse([H | T], Acc) -> reverse(T, [H | Acc]).

%% The run lengths of the tags in arrival order: {Tag, Count} for each run.
runs([]) -> [];
runs([Tag | Tags]) -> runs(Tags, Tag, 1).

runs([Tag | Tags], Tag, N) -> runs(Tags, Tag, N + 1);
runs(Tags, Tag, N) -> [{Tag, N} | runs(Tags)].
