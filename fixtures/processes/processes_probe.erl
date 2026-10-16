-module(processes_probe).
-export([main/0, alone/0, wait/0, reductions/0, exited/1]).

%% Return the erlang:processes/0 and process_info(Pid, group_leader)
%% results that do not depend on the other processes of the node:
%% processes/0 lists the caller and a live child in pid order
%% (erl_ptab.c:746-780), and not a process that exited. A child has the
%% group leader of its parent (erl_bif_info.c:2107-2112), and a dead
%% process gives undefined.
main() ->
    Self = self(),
    Child = spawn(?MODULE, wait, []),
    {Dead, Ref} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', Ref, process, Dead, normal} -> ok end,
    Ps = processes(),
    Result = [lists:member(Self, Ps), lists:member(Child, Ps), lists:member(Dead, Ps),
              Ps =:= lists:sort(Ps),
              process_info(Child, group_leader) =:= {group_leader, group_leader()},
              process_info(Self, group_leader) =:= {group_leader, group_leader()},
              process_info(Dead, group_leader)],
    Child ! stop,
    Result.

%% Return true when processes/0 gives only the caller. In a PBeam run
%% with a policy module, the run also has the policy process, which a
%% process that is not a boot process does not get.
alone() ->
    processes() =:= [self()].

wait() ->
    receive stop -> ok end.

%% Return the reductions of one processes/0 call and the two
%% process_info/2 calls around it. ERTS reads the process table of
%% 1048576 indices in chunks of 1000 and charges 1 reduction for each 25
%% indices (erl_ptab.c:42-43, 1022-1023), so the call traps about 10 times.
reductions() ->
    {reductions, A} = process_info(self(), reductions),
    true = is_list(processes()),
    {reductions, B} = process_info(self(), reductions),
    B - A.

%% Call processes/0 with about Left reductions in the turn, after the spawn
%% of a child that exits at once and of a keeper that waits. The 1000
%% processes before them put both in the second chunk of the table. Return
%% whether the result has the child, and whether the child is last: a child
%% that exits before the listing reads its chunk comes after the keeper,
%% from the list of deleted processes (erl_ptab.c:1052-1170). With fewer than
%% 40 reductions, the call traps before it reads the table
%% (erl_ptab.c:766-768), and the child exits before the listing starts.
exited(Left) ->
    [receive {'DOWN', Ref, process, Pid, normal} -> ok end
     || {Pid, Ref} <- [spawn_monitor(fun() -> ok end) || _ <- lists:seq(1, 1000)]],
    erlang:yield(),
    erlang:bump_reductions(4000 - Left),
    Child = spawn(fun() -> ok end),
    Keeper = spawn(?MODULE, wait, []),
    Ps = processes(),
    Keeper ! stop,
    {lists:member(Child, Ps), lists:last(Ps) =:= Child}.
