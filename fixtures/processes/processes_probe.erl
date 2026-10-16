-module(processes_probe).
-export([main/0, alone/0, wait/0]).

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
