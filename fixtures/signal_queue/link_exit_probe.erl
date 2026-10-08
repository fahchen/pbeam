-module(link_exit_probe).
-compile([export_all, nowarn_export_all]).

%% Each case runs in a new process. The result is {Case, Value}, or
%% {Case, {died, Reason}} when the case process exits.
all() ->
    [{Case, run(Case)} || Case <- cases()].

cases() ->
    [chain, chain_kill, chain_normal, exit_kill_links, busy_linked, untrapped,
     link_dead, link_dead_trap, link_dead_order, link_dead_untrap, link_exiting,
     link_exiting_trap, link_self].

run(Case) ->
    Self = self(),
    {P, R} = spawn_monitor(fun() -> Self ! {result, self(), ?MODULE:Case()} end),
    receive
        {result, P, Value} -> receive {'DOWN', R, _, _, _} -> Value end;
        {'DOWN', R, _, _, Reason} -> {died, Reason}
    end.

flush() -> receive M -> [M | flush()] after 0 -> [] end.

%% A process that links to the pids of {link, Pid} and exits with the
%% reason of {exit, Reason}.
member() ->
    receive
        {link, P, From} -> link(P), From ! linked, member();
        {exit, Reason} -> exit(Reason)
    end.

busy() -> busy().

%% Start a member, monitor it, and give it the name Name in the result.
start(Name) ->
    P = spawn(?MODULE, member, []),
    monitor(process, P),
    {Name, P}.

connect({_, A}, {_, B}) ->
    A ! {link, B, self()},
    receive linked -> ok end.

dead() ->
    {P, R} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', R, process, P, _} -> P end.

%% Read the messages of the case process until each member is down, in
%% arrival order, with the names of the members.
collect(Members) -> collect(Members, length(Members)).

collect(_, 0) -> [];
collect(Members, N) ->
    receive
        {'DOWN', _, process, P, R} -> [{down, name(P, Members), R} | collect(Members, N - 1)];
        {'EXIT', P, R} -> [{exit, name(P, Members), R} | collect(Members, N)]
    end.

name(P, Members) ->
    {Name, P} = lists:keyfind(P, 2, Members),
    Name.

%% The case process traps exits and links A. A links B, and B links C.
%% C exits with Reason. The exit signal kills B and A, which do not trap
%% exits, and the case process gets {'EXIT', A, Reason}.
chain() -> chain_of(boom).
chain_kill() -> chain_of(kill).
chain_normal() ->
    process_flag(trap_exit, true),
    [A, B, C] = [start(a), start(b), start(c)],
    link(element(2, A)),
    connect(A, B),
    connect(B, C),
    element(2, C) ! {exit, normal},
    Down = receive {'DOWN', _, process, _, R} -> R end,
    {Down, is_process_alive(element(2, A)), is_process_alive(element(2, B)), flush()}.

chain_of(Reason) ->
    process_flag(trap_exit, true),
    Members = [A, B, C] = [start(a), start(b), start(c)],
    link(element(2, A)),
    connect(A, B),
    connect(B, C),
    element(2, C) ! {exit, Reason},
    collect(Members).

%% exit(B, kill) gives killed to B, and B sends killed through its link
%% to C, which does not trap exits.
exit_kill_links() ->
    process_flag(trap_exit, true),
    Members = [B, C] = [start(b), start(c)],
    link(element(2, B)),
    connect(B, C),
    exit(element(2, B), kill),
    collect(Members).

%% B runs a loop with no receive when the exit signal of C arrives.
busy_linked() ->
    process_flag(trap_exit, true),
    C = start(c),
    B = spawn(fun() -> link(element(2, C)), busy() end),
    monitor(process, B),
    link(B),
    erlang:yield(),
    element(2, C) ! {exit, boom},
    collect([{b, B}, C]).

%% The case process does not trap exits, and the exit of a linked process
%% ends it.
untrapped() ->
    C = start(c),
    link(element(2, C)),
    element(2, C) ! {exit, boom},
    receive never -> ok end.

link_dead() ->
    P = dead(),
    try link(P) catch Class:Reason -> {Class, Reason} end.

%% With trap_exit, link/1 to a dead pid returns true, and the exit signal
%% noproc comes after the messages that the caller sent before.
link_dead_trap() ->
    process_flag(trap_exit, true),
    P = dead(),
    true = link(P),
    [{'EXIT', P, R}] = flush(),
    R.

link_dead_order() ->
    process_flag(trap_exit, true),
    P = dead(),
    self() ! a,
    true = link(P),
    self() ! b,
    [case M of {'EXIT', P, R} -> {'EXIT', R}; _ -> M end || M <- flush()].

%% The caller handles the noproc signal after it stops to trap exits, so
%% the signal ends it.
link_dead_untrap() ->
    process_flag(trap_exit, true),
    P = dead(),
    true = link(P),
    process_flag(trap_exit, false),
    flush().

%% The target handles the kill signal after the link signal is sent, so
%% link/1 returns true. The target answers the link signal with noproc at
%% its exit.
link_exiting() ->
    P = spawn(?MODULE, member, []),
    exit(P, kill),
    link(P),
    receive never -> ok end.

link_exiting_trap() ->
    process_flag(trap_exit, true),
    P = spawn(?MODULE, member, []),
    exit(P, kill),
    true = link(P),
    receive {'EXIT', P, R} -> R end.

link_self() ->
    {link(self()), flush()}.
