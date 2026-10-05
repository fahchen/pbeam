-module(exit_probe).
-export([main/0, self_exit/2, target/1, linked/0]).

main() ->
    [self_case(false, normal), self_case(false, boom), self_case(false, kill),
     self_case(true, normal), self_case(true, boom), self_case(true, kill),
     other(false, normal), other(false, boom), other(false, kill),
     other(true, normal), other(true, kill),
     catch_badarg(foo), catch_badarg(1), exit(make_ref(), boom), dead(), linked_case()].

self_case(Trap, Reason) ->
    {Pid, Ref} = spawn_monitor(?MODULE, self_exit, [Trap, Reason]),
    receive {'DOWN', Ref, process, Pid, Why} -> Why end.

self_exit(Trap, Reason) ->
    process_flag(trap_exit, Trap),
    R = try exit(self(), Reason) catch Class:Error -> {Class, Error} end,
    Msg = receive M -> M after 0 -> none end,
    exit({returned, R, Msg =:= {'EXIT', self(), Reason}}).

other(Trap, Reason) ->
    {Pid, Ref} = spawn_monitor(?MODULE, target, [Trap]),
    Pid ! {ready, self()}, receive ready -> ok end,
    true = exit(Pid, Reason),
    Pid ! report,
    receive {'DOWN', Ref, process, Pid, Why} -> Why end.

target(Trap) ->
    process_flag(trap_exit, Trap),
    receive {ready, P} -> P ! ready end,
    receive report -> ok end,
    Msgs = receive {'EXIT', _, R} -> R after 0 -> none end,
    exit({alive, Msgs}).

catch_badarg(X) -> try exit(X, boom) catch error:E -> E end.

dead() ->
    {Pid, Ref} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', Ref, process, Pid, _} -> ok end,
    exit(Pid, boom).

linked_case() ->
    {Pid, Ref} = spawn_monitor(?MODULE, linked, []),
    receive {'DOWN', Ref, process, Pid, Why} -> Why end.

linked() ->
    process_flag(trap_exit, true),
    Pid = spawn_link(?MODULE, target, [false]),
    true = exit(Pid, boom),
    exit(receive {'EXIT', Pid, Why} -> {linked, Why} end).
