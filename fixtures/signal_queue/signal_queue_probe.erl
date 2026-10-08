-module(signal_queue_probe).
-compile([export_all, nowarn_export_all]).

%% Each case runs in a new process. The result is {Case, Value}, or
%% {Case, {died, Reason}} when the case process exits.
all() ->
    [{Case, run(Case)} || Case <- cases()].

cases() ->
    [demonitor_dead, demonitor_dead_received, demonitor_name, exit_then_self_msg,
     exit_then_monitor, exit_then_link_trap, unlink_after_exit, unlink_after_exit_trap,
     alive_after_exit, info_after_exit, gl_after_exit, gl_after_msg, gl_after_link,
     gl_order, gl_set_order, link_order, monitor_order, monitor_message_order,
     demonitor_order, exit_trap_order, exit_normal_order, unlink_order, link_again_order,
     spawn_link_order, monitor_dead_order, sender_order, down_after_exit, alias_after_down,
     alias_kill, alias_exit_then_kill, alive_after_monitor, alive_self_message,
     alive_no_message, alive_trap, info_self_message, info_request, monitor_then_kill,
     demonitor_then_kill, alias_self_flush, kill_runnable, unlink_dead, self_kill, self_boom,
     self_normal, self_trap, alias_alive_request, alive_request_exit, alive_request_dead,
     gl_request_dead, request_badargs, exiting_target, reply_to_exiting, unlink_unhandled_exit,
     hibernate_signals, pending_replace, alias_to_dead_owner, info_request_backlog, marker_exit,
     flush_exit, flush_yield, return_exit, return_yield, await_yield, receive_yield].

run(Case) ->
    Self = self(),
    {P, R} = spawn_monitor(fun() -> Self ! {result, self(), ?MODULE:Case()} end),
    receive
        {result, P, Value} -> receive {'DOWN', R, _, _, _} -> Value end;
        {'DOWN', R, _, _, Reason} -> {died, Reason}
    end.

idle() -> receive stop -> ok end.

reporter(Parent, Name) ->
    Parent ! {ready, self()},
    receive go -> Parent ! Name end.

gl_target() ->
    receive _ -> ok end,
    receive stop -> ok end.

ready(P) -> receive {ready, P} -> ok end.

flush() -> receive M -> [M | flush()] after 0 -> [] end.

wait_down(Ref) -> receive {'DOWN', Ref, _, _, Reason} -> Reason end.

collect(0) -> [];
collect(N) -> receive X -> [X | collect(N - 1)] end.

strip({'EXIT', P, R}, P) -> {'EXIT', R};
strip({'DOWN', _, process, P, R}, P) -> {'DOWN', R};
strip(M, _) -> M.

dead() ->
    {P, R} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', R, process, P, _} -> P end.

%% Two waiting reporters B and C. Control(B) runs, then C gets go and B
%% gets go. The order of their reports shows which process ran first.
order(Control) ->
    Self = self(),
    B = spawn(?MODULE, reporter, [Self, b]),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(B),
    ready(C),
    Control(B),
    C ! go,
    B ! go,
    collect(2).

%% A monitor of a dead pid: demonitor/2 finds the monitor, because the
%% DOWN signal is not handled yet.
demonitor_dead() ->
    P = dead(),
    M = monitor(process, P),
    R = demonitor(M, [info]),
    {R, flush()}.

demonitor_dead_received() ->
    P = dead(),
    M = monitor(process, P),
    D = receive {'DOWN', M, process, P, Why} -> Why end,
    {D, demonitor(M, [info]), flush()}.

demonitor_name() ->
    M = monitor(process, signal_queue_nobody),
    R = demonitor(M, [info]),
    {R, flush()}.

exit_then_self_msg() ->
    process_flag(trap_exit, true),
    P = spawn_link(?MODULE, idle, []),
    Ref = monitor(process, P),
    exit(P, kill),
    self() ! after_exit,
    D = wait_down(Ref),
    Ms = flush(),
    process_flag(trap_exit, false),
    {D, [strip(M, P) || M <- Ms]}.

exit_then_monitor() ->
    P = spawn(?MODULE, idle, []),
    exit(P, kill),
    M = monitor(process, P),
    {wait_down(M), flush()}.

exit_then_link_trap() ->
    process_flag(trap_exit, true),
    P = spawn(?MODULE, idle, []),
    Ref = monitor(process, P),
    exit(P, kill),
    L = link(P),
    D = wait_down(Ref),
    E = receive {'EXIT', P, Why} -> {'EXIT', Why} after 100 -> no_exit end,
    process_flag(trap_exit, false),
    {L, D, E, flush()}.

unlink_after_exit() ->
    process_flag(trap_exit, true),
    P = spawn_link(?MODULE, idle, []),
    Ref = monitor(process, P),
    exit(P, boom),
    U = unlink(P),
    D = wait_down(Ref),
    Ms = flush(),
    process_flag(trap_exit, false),
    {U, D, [strip(M, P) || M <- Ms]}.

unlink_after_exit_trap() ->
    process_flag(trap_exit, true),
    Self = self(),
    P = spawn_link(fun() ->
                           process_flag(trap_exit, true),
                           Self ! {ready, self()},
                           receive M -> Self ! {got, M} end,
                           receive stop -> ok end
                   end),
    ready(P),
    exit(P, boom),
    U = unlink(P),
    G = receive {got, X} -> X end,
    P ! stop,
    {U, strip(G, Self), flush()}.

alive_after_exit() ->
    P = spawn(?MODULE, idle, []),
    exit(P, kill),
    is_process_alive(P).

info_after_exit() ->
    P = spawn(?MODULE, idle, []),
    exit(P, kill),
    process_info(P, registered_name).

gl_after_exit() ->
    P = spawn(?MODULE, idle, []),
    exit(P, kill),
    try group_leader(self(), P) catch error:Reason -> Reason end.

gl_after_msg() ->
    P = spawn(?MODULE, gl_target, []),
    P ! hello,
    R = erts_internal:group_leader(self(), P),
    P ! stop,
    R.

gl_after_link() ->
    P = spawn(?MODULE, idle, []),
    erlang:yield(),
    link(P),
    R = erts_internal:group_leader(self(), P),
    unlink(P),
    P ! stop,
    R.

%% group_leader/2 to a process with a message that it did not handle
%% sends a signal and waits for the reply, so C runs first.
gl_order() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    P = spawn(?MODULE, gl_target, []),
    erlang:yield(),
    C ! go,
    P ! hello,
    true = group_leader(Self, P),
    Self ! after_gl,
    R = collect(2),
    P ! stop,
    R.

gl_set_order() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    P = spawn(?MODULE, gl_target, []),
    erlang:yield(),
    C ! go,
    true = group_leader(Self, P),
    Self ! after_gl,
    R = collect(2),
    P ! stop,
    R.

%% A control signal puts a waiting process in the run queue at once.
link_order() -> order(fun(B) -> link(B) end).

%% A monitor signal waits in the pending slot until the sender switches out.
monitor_order() -> order(fun(B) -> monitor(process, B) end).

monitor_message_order() ->
    Self = self(),
    B = spawn(?MODULE, reporter, [Self, b]),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(B),
    ready(C),
    monitor(process, B),
    C ! go,
    B ! go,
    collect(2).

demonitor_order() ->
    order(fun(B) ->
                  M = monitor(process, B),
                  erlang:yield(),
                  demonitor(M)
          end).

exit_trap_order() ->
    Self = self(),
    B = spawn(fun() ->
                      process_flag(trap_exit, true),
                      Self ! {ready, self()},
                      receive {'EXIT', _, _} -> Self ! b end
              end),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(B),
    ready(C),
    exit(B, hello),
    C ! go,
    collect(2).

exit_normal_order() -> order(fun(B) -> exit(B, normal) end).

unlink_order() -> order(fun(B) -> unlink(B) end).

link_again_order() -> order(fun(B) -> link(B), erlang:yield() end).

spawn_link_order() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    C ! go,
    B = spawn_link(fun() -> Self ! b end),
    R = collect(2),
    unlink(B),
    R.

monitor_dead_order() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    D = dead(),
    C ! go,
    M = monitor(process, D),
    [case X of {'DOWN', M, process, D, W} -> {'DOWN', W}; _ -> X end || X <- collect(2)].

%% Messages and control signals from one sender keep their order.
sender_order() ->
    process_flag(trap_exit, true),
    Self = self(),
    P = spawn(fun() -> Self ! m1, exit(Self, e1), Self ! m2, link(Self), exit(boom) end),
    Ref = monitor(process, P),
    wait_down(Ref),
    Ms = flush(),
    process_flag(trap_exit, false),
    [strip(M, P) || M <- Ms].

%% The link exit signal comes before the monitor down signal.
down_after_exit() ->
    process_flag(trap_exit, true),
    P = spawn(?MODULE, idle, []),
    M = monitor(process, P),
    link(P),
    P ! stop,
    D = wait_down(M),
    Ms = flush(),
    process_flag(trap_exit, false),
    {D, [strip(X, P) || X <- Ms]}.

%% A message through a demonitor alias after the target exited, before the
%% owner handles the DOWN signal: the owner drops it.
alias_after_down() ->
    Self = self(),
    T = spawn(?MODULE, idle, []),
    A = monitor(process, T, [{alias, demonitor}]),
    R = spawn(fun() -> receive go -> A ! via_alias, Self ! sent end end),
    T ! stop,
    R ! go,
    erlang:yield(),
    erlang:yield(),
    erlang:yield(),
    Ms = receive sent -> flush() end,
    [case M of {'DOWN', A, process, T, Rs} -> {'DOWN', Rs}; _ -> M end || M <- Ms].

%% A process ends itself through its own alias; it exits when it handles
%% the signal in its receive.
alias_kill() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    T = spawn(?MODULE, idle, []),
    {B, M} = spawn_monitor(fun() ->
                                   A = monitor(process, T, [{alias, demonitor}]),
                                   true = exit(A, kill),
                                   C ! go,
                                   receive never -> ok end
                           end),
    R = collect(2),
    T ! stop,
    [case X of {'DOWN', M, process, B, W} -> {'DOWN', W}; _ -> X end || X <- R].

alias_exit_then_kill() ->
    T = spawn(?MODULE, idle, []),
    {B, M} = spawn_monitor(fun() ->
                                   A = monitor(process, T, [{alias, demonitor}]),
                                   true = exit(A, boom),
                                   exit(self(), kill)
                           end),
    R = receive {'DOWN', M, process, B, W} -> W end,
    T ! stop,
    R.

alive_after_monitor() ->
    P = spawn(?MODULE, idle, []),
    monitor(process, P),
    R = is_process_alive(P),
    P ! stop,
    R.

%% is_process_alive/1 with a message to the caller traps to handle the
%% signals of the caller, and the trap switches out the caller.
alive_self_message() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    B = spawn(?MODULE, idle, []),
    erlang:yield(),
    C ! go,
    Self ! x,
    true = is_process_alive(B),
    Self ! y,
    R = collect(3),
    B ! stop,
    R.

alive_no_message() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    B = spawn(?MODULE, idle, []),
    erlang:yield(),
    C ! go,
    true = is_process_alive(B),
    Self ! y,
    R = collect(2),
    B ! stop,
    R.

alive_trap() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    B = spawn(?MODULE, idle, []),
    erlang:yield(),
    link(B),
    C ! go,
    true = is_process_alive(B),
    Self ! y,
    R = collect(2),
    unlink(B),
    B ! stop,
    R.

info_self_message() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    B = spawn(?MODULE, idle, []),
    erlang:yield(),
    C ! go,
    Self ! x,
    [] = process_info(B, registered_name),
    Self ! y,
    R = collect(3),
    B ! stop,
    R.

%% process_info/2 of a process with a message that it did not handle sends
%% a request and waits for the reply.
info_request() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    B = spawn(?MODULE, gl_target, []),
    erlang:yield(),
    C ! go,
    B ! hello,
    L = process_info(B, message_queue_len),
    Self ! y,
    R = collect(2),
    B ! stop,
    {L, R}.

monitor_then_kill() ->
    P = spawn(?MODULE, idle, []),
    erlang:yield(),
    M = monitor(process, P),
    exit(P, kill),
    wait_down(M).

demonitor_then_kill() ->
    P = spawn(?MODULE, idle, []),
    erlang:yield(),
    M = monitor(process, P),
    demonitor(M),
    exit(P, kill),
    M2 = monitor(process, P),
    receive {'DOWN', M2, process, P, _} -> ok end,
    flush().

alias_self_flush() ->
    P = spawn(?MODULE, idle, []),
    A = monitor(process, P, [{alias, demonitor}]),
    A ! m,
    D = demonitor(A, [flush]),
    P ! stop,
    {D, flush()}.

%% An exit signal to a process in the run queue: it exits at schedule-in,
%% before C runs.
kill_runnable() ->
    process_flag(trap_exit, true),
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    B = spawn_link(?MODULE, idle, []),
    erlang:yield(),
    B ! wake,
    C ! go,
    exit(B, kill),
    R = collect(2),
    process_flag(trap_exit, false),
    [strip(X, B) || X <- R].

unlink_dead() ->
    process_flag(trap_exit, true),
    P = spawn_link(fun() -> ok end),
    receive {'EXIT', P, normal} -> ok end,
    U = unlink(P),
    process_flag(trap_exit, false),
    U.

self_exit(Reason) ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    {A, M} = spawn_monitor(fun() -> C ! go, exit(self(), Reason) end),
    [case X of {'DOWN', M, process, A, W} -> {'DOWN', W}; _ -> X end || X <- collect(2)].

self_kill() -> self_exit(kill).

self_boom() -> self_exit(boom).

self_normal() -> self_exit(normal).

self_trap() ->
    Self = self(),
    C = spawn(?MODULE, reporter, [Self, c]),
    ready(C),
    {A, M} = spawn_monitor(fun() ->
                                   process_flag(trap_exit, true),
                                   C ! go,
                                   R = exit(self(), boom),
                                   Self ! {ret, R},
                                   receive E -> Self ! E end
                           end),
    [case X of
         {'DOWN', M, process, A, W} -> {'DOWN', W};
         {'EXIT', A, W} -> {'EXIT', W};
         _ -> X
     end || X <- collect(4)].

%% erts_internal:is_process_alive/2 does not take an alias.
alias_alive_request() ->
    P = spawn(?MODULE, idle, []),
    A = monitor(process, P, [{alias, demonitor}]),
    R = try erts_internal:is_process_alive(P, A) catch error:Reason -> Reason end,
    P ! stop,
    R.

%% An is-alive request that waits behind a kill signal gets false when the
%% target exits.
alive_request_exit() ->
    P = spawn(?MODULE, idle, []),
    exit(P, kill),
    Ref = make_ref(),
    ok = erts_internal:is_process_alive(P, Ref),
    receive {Ref, R} -> R end.

alive_request_dead() ->
    P = dead(),
    Ref = make_ref(),
    ok = erts_internal:is_process_alive(P, Ref),
    A = receive {Ref, R} -> R end,
    {A, is_process_alive(P)}.

gl_request_dead() ->
    P = dead(),
    Ref = make_ref(),
    ok = erts_internal:group_leader(self(), P, Ref),
    receive {Ref, R} -> R end.

request_badargs() ->
    Error = fun(F) -> try F() catch error:Reason -> Reason end end,
    [Error(fun() -> erts_internal:group_leader(self(), x, make_ref()) end),
     Error(fun() -> erts_internal:group_leader(x, self(), make_ref()) end),
     Error(fun() -> erts_internal:is_process_alive(x, make_ref()) end),
     Error(fun() -> is_process_alive(x) end),
     erts_internal:group_leader(x, self()),
     erts_internal:group_leader(self(), x)].

%% P ends itself through its own alias in its receive and exits in its
%% next turn. In between, P exits: group_leader/2 gives badarg, a message
%% to P is dropped, and process_info/2 sends a request.
exiting_target() ->
    Self = self(),
    T = spawn(?MODULE, idle, []),
    P = spawn(fun() ->
                      A = monitor(process, T, [{alias, demonitor}]),
                      true = exit(A, kill),
                      Self ! {ready, self()},
                      receive never -> ok end
              end),
    ready(P),
    GL = erts_internal:group_leader(self(), P),
    P ! dropped,
    Info = process_info(P, registered_name),
    T ! stop,
    {GL, Info}.

%% R sends an is-alive request to T and then exits in its next turn, so the
%% reply finds R exiting and is dropped.
reply_to_exiting() ->
    T = spawn(?MODULE, idle, []),
    erlang:yield(),
    {_, M} = spawn_monitor(fun() ->
                                   ok = erts_internal:is_process_alive(T, make_ref()),
                                   A = monitor(process, T, [{alias, demonitor}]),
                                   true = exit(A, kill),
                                   receive never -> ok end
                           end),
    W = wait_down(M),
    T ! stop,
    W.

repeat(0, _) -> ok;
repeat(N, F) -> F(), repeat(N - 1, F).

spin(0) -> ok;
spin(N) -> erlang:yield(), spin(N - 1).

%% Spawn a process that sends N exit signals normal to Target, which ignores
%% them, and then runs Then().
flood(Target, N, Then) ->
    spawn(fun() -> repeat(N, fun() -> exit(Target, normal) end), Then() end).

%% The link exit signal of P waits behind other signals, so unlink/1 finds
%% an active link to a process that does not exist and removes it at once.
unlink_unhandled_exit() ->
    Self = self(),
    P = spawn_link(fun() -> receive go -> ok end end),
    flood(Self, 800, fun() -> P ! go end),
    spin(2),
    U = unlink(P),
    {U, flush()}.

%% B hibernates with signals and a message in its middle queue part. The
%% handling at its next schedule-in moves the message into its inner part
%% and makes it active.
hibernate_signals() ->
    Self = self(),
    B = spawn(fun() ->
                      receive go -> ok end,
                      erlang:hibernate(),
                      receive M -> Self ! {got, M} end
              end),
    erlang:yield(),
    B ! go,
    repeat(800, fun() -> exit(B, normal) end),
    B ! m,
    receive {got, X} -> X end.

%% A second monitor signal replaces the pending one, which then goes to a
%% process that does not exist.
pending_replace() ->
    D = dead(),
    P = spawn(?MODULE, idle, []),
    M1 = monitor(process, D),
    M2 = monitor(process, P),
    R = wait_down(M1),
    P ! stop,
    {R, wait_down(M2)}.

alias_to_dead_owner() ->
    Self = self(),
    T = spawn(?MODULE, idle, []),
    {O, M} = spawn_monitor(fun() -> Self ! {alias, monitor(process, T, [{alias, demonitor}])} end),
    A = receive {alias, X} -> X end,
    wait_down(M),
    A ! dropped,
    T ! stop,
    {is_process_alive(O), flush()}.

%% process_info/2 of a busy process from a caller with signals in its
%% middle queue part: the save pointer passes the queue through a marker.
info_request_backlog() ->
    Self = self(),
    B = spawn(?MODULE, gl_target, []),
    erlang:yield(),
    flood(Self, 800, fun() -> ok end),
    spin(2),
    B ! hello,
    L = process_info(B, message_queue_len),
    B ! stop,
    L.

%% A receive marker in the middle queue part is still there when the
%% caller exits.
marker_exit() ->
    Self = self(),
    flood(Self, 800, fun() -> exit(Self, kill) end),
    spin(2),
    Ref = make_ref(),
    receive {Ref, _} -> ok end.

%% The flush of process_info/2 of the caller reaches a kill signal.
flush_exit() ->
    Self = self(),
    flood(Self, 800, fun() -> exit(Self, kill) end),
    spin(2),
    Self ! x,
    process_info(Self, message_queue_len).

%% The flush of process_info/2 of the caller uses its reductions and
%% continues in the next turns, also with all reductions at schedule-in.
flush_yield() ->
    Self = self(),
    flood(Self, 20000, fun() -> ok end),
    spin(25),
    Self ! x,
    process_info(Self, message_queue_len).

%% is_process_alive/1 traps to handle the signals of the caller, which
%% reach a kill signal.
return_exit() ->
    Self = self(),
    B = spawn(?MODULE, idle, []),
    erlang:yield(),
    flood(Self, 800, fun() -> exit(Self, kill) end),
    spin(2),
    is_process_alive(B).

return_yield() ->
    Self = self(),
    B = spawn(?MODULE, idle, []),
    erlang:yield(),
    flood(Self, 8000, fun() -> ok end),
    spin(10),
    R = is_process_alive(B),
    B ! stop,
    R.

%% exit(self(), kill) waits behind many signals.
await_yield() ->
    Self = self(),
    flood(Self, 8000, fun() -> ok end),
    spin(10),
    exit(Self, kill).

%% The receive helper uses the reductions of the turn to handle signals,
%% and loop_rec runs again in the next turn.
receive_yield() ->
    Self = self(),
    flood(Self, 8000, fun() -> Self ! done end),
    spin(10),
    receive done -> ok end.
