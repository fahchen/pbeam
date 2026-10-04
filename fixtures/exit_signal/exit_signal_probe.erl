-module(exit_signal_probe).
-export([main/0, crash/0, finish/0]).
main() ->
    process_flag(trap_exit, true),
    {order(), normal_exit()}.
order() ->
    {Pid, _Ref} = spawn_monitor(?MODULE, crash, []),
    true = link(Pid),
    Pid ! go,
    First = receive M1 -> M1 end,
    Second = receive M2 -> M2 end,
    {exit_reason(First), down_reason(Second)}.
normal_exit() ->
    Pid = spawn_link(?MODULE, finish, []),
    Pid ! go,
    receive {'EXIT', Pid, R} -> {'EXIT', R} end.
crash() -> receive go -> exit(boom) end.
finish() -> receive go -> ok end.
exit_reason({'EXIT', _, R}) -> {'EXIT', R};
exit_reason(_) -> other.
down_reason({'DOWN', _, process, _, R}) -> {'DOWN', R};
down_reason(_) -> other.
