-module(exit_probe).
-export([main/0, self_exit/2, target/1, linked/0, exit3/0, self_exit3/3]).

main() ->
    [self_case(false, normal), self_case(false, boom), self_case(false, kill),
     self_case(true, normal), self_case(true, boom), self_case(true, kill),
     other(false, normal), other(false, boom), other(false, kill),
     other(true, normal), other(true, kill),
     catch_badarg(foo), catch_badarg(1), exit(make_ref(), boom), resource(), dead(),
     linked_case()].

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
    Msgs = receive {'EXIT', From, R} -> {R, From =:= P} after 0 -> none end,
    exit({alive, Msgs}).

catch_badarg(X) -> try exit(X, boom) catch error:E -> E end.

%% A NIF resource is a magic reference in ERTS, and is_internal_ref/1 is
%% true for it (erl_term.h:1064-1065), so exit/2 gives true and no badarg.
resource() ->
    {ok, {re_pattern, _, _, _, Ref}} = re:compile(<<"a">>, []),
    exit(Ref, boom).

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

%% erlang:exit/3 (bif.c:1877-1880): Options is a proper list of the atom
%% priority. Other options raise badarg with the cause badopt before Id is
%% checked (bif.c:1727-1744). The signal is the signal of exit/2.
exit3() ->
    [self3(false, normal, [priority]), self3(true, normal, [priority]),
     self3(true, kill, []),
     other3(true, boom, [priority]), other3(false, normal, [priority, priority]),
     other3(false, kill, []),
     exit(dead_pid(), boom, [priority]),
     cause(self(), [x]), cause(self(), [{priority, true}]), cause(self(), priority),
     cause(self(), [priority | x]), cause(foo, [bad]), cause(foo, [priority]),
     apply(erlang, exit, [make_ref(), boom, [priority]])].

self3(Trap, Reason, Options) ->
    {Pid, Ref} = spawn_monitor(?MODULE, self_exit3, [Trap, Reason, Options]),
    receive {'DOWN', Ref, process, Pid, Why} -> Why end.

self_exit3(Trap, Reason, Options) ->
    process_flag(trap_exit, Trap),
    R = exit(self(), Reason, Options),
    Msg = receive M -> M after 0 -> none end,
    exit({returned, R, Msg =:= {'EXIT', self(), Reason}}).

other3(Trap, Reason, Options) ->
    {Pid, Ref} = spawn_monitor(?MODULE, target, [Trap]),
    Pid ! {ready, self()}, receive ready -> ok end,
    true = exit(Pid, Reason, Options),
    Pid ! report,
    receive {'DOWN', Ref, process, Pid, Why} -> Why end.

cause(Id, Options) ->
    try exit(Id, boom, Options)
    catch error:E:Stack ->
        [{erlang, exit, [Id, boom, Options], [{error_info, Info}]} | _] = Stack,
        #{module := erl_erts_errors} = Info,
        {E, case Info of #{cause := Cause} -> Cause; #{} -> none end}
    end.

dead_pid() ->
    {Pid, Ref} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', Ref, process, Pid, _} -> Pid end.
