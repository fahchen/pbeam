-module(signal_reds_probe).
-export([all/0, idle/0, trapper/0, two/0, marker/0, exiter/0, watcher/1]).

%% The reductions that a process B uses to handle the signals that the
%% caller sends to it. Each measure reads process_info(B, reductions)
%% before and after, and B does not run Erlang code between, except where
%% a case says so.
all() ->
    [{none, measure(idle, fun(_) -> ok end)},
     {link, measure(idle, fun(B) -> link(B) end)},
     {link_unlink_10, measure(idle, fun(B) -> repeat(10, fun() -> link(B), unlink(B) end) end)},
     {link_10, measure(idle, fun(B) -> repeat(10, fun() -> link(B) end) end)},
     {monitor, measure(idle, fun(B) -> monitor(process, B), self() ! x, receive x -> ok end end)},
     {monitor_10, measure(idle, fun(B) -> repeat(10, fun() -> monitor(process, B) end) end)},
     {monitor_demonitor_10,
      measure(idle, fun(B) -> repeat(10, fun() -> demonitor(monitor(process, B)) end) end)},
     {exit_normal, measure(idle, fun(B) -> exit(B, normal) end)},
     {exit_normal_100, measure(idle, fun(B) -> repeat(100, fun() -> exit(B, normal) end) end)},
     {exit_normal_500, measure(idle, fun(B) -> repeat(500, fun() -> exit(B, normal) end) end)},
     %% B traps exits, so it also receives the EXIT message: 1 for the
     %% handling, 4 for the fetch in loop_rec, and 1 for loop_rec_end.
     {exit_trap, measure(trapper, fun(B) -> exit(B, hello) end)},
     {exit_trap_10, measure(trapper, fun(B) -> repeat(10, fun() -> exit(B, hello) end) end)},
     {group_leader, measure(idle, fun(B) -> B ! m, group_leader(self(), B) end)},
     {is_process_alive, measure(idle, fun(B) -> link(B), is_process_alive(B) end)},
     {process_info, measure(idle, fun(B) -> link(B), process_info(B, registered_name) end)},
     {unlink_ack, unlink_ack()},
     %% B gets wake, reserves a receive marker behind the exit signals, and
     %% passes the marker in its receive.
     {marker, measure(marker, fun(B) -> B ! wake, repeat(300, fun() -> exit(B, normal) end) end)},
     %% A DOWN message with an immediate reason costs 8 more than one with
     %% another reason.
     {down, [{Reason, down(Reason)} || Reason <- [normal, 7, [], [a], {shutdown, x}]]},
     %% B handles a process-info request in loop_rec, so the reductions of
     %% its current turn count.
     {info_running, info_running()},
     {active, [{N, active(N)} || N <- [1, 10, 133, 134, 135, 300, 1000, 6000]]},
     {waiting, [{N, waiting(N)} || N <- [1, 1000, 6000]]}].

idle() -> receive stop -> ok end.

trapper() -> process_flag(trap_exit, true), receive stop -> ok end.

two() -> receive wake -> ok end, receive stop -> ok end.

marker() ->
    receive wake -> ok end,
    Ref = make_ref(),
    self() ! {Ref, ok},
    receive {Ref, ok} -> ok end,
    receive stop -> ok end.

exiter() -> receive {exit, Reason} -> exit(Reason) end.

watcher(T) ->
    monitor(process, T),
    receive {'DOWN', _, _, _, _} -> ok end,
    receive stop -> ok end.

info_running() ->
    B = spawn(?MODULE, two, []),
    settle(),
    B ! wake,
    repeat(800, fun() -> exit(B, normal) end),
    R = reds(B),
    B ! stop,
    R.

%% The reductions of a watcher W for the DOWN message of T.
down(Reason) ->
    T = spawn(?MODULE, exiter, []),
    W = spawn(?MODULE, watcher, [T]),
    settle(),
    R0 = reds(W),
    T ! {exit, Reason},
    settle(),
    R1 = reds(W),
    W ! stop,
    R1 - R0.

repeat(0, _) -> ok;
repeat(N, F) -> F(), repeat(N - 1, F).

reds(P) -> {reductions, R} = process_info(P, reductions), R.

settle() -> erlang:yield(), erlang:yield().

measure(Start, Act) ->
    B = spawn(?MODULE, Start, []),
    settle(),
    R0 = reds(B),
    Act(B),
    settle(),
    R1 = reds(B),
    B ! stop,
    R1 - R0.

%% The reductions of the caller for the unlink acknowledgment: the handling
%% at schedule-in after a yield, and then in loop_rec.
unlink_ack() ->
    B = spawn(?MODULE, idle, []),
    settle(),
    link(B),
    settle(),
    R0 = reds(self()),
    unlink(B),
    settle(),
    R1 = reds(self()),
    receive after 0 -> ok end,
    R2 = reds(self()),
    B ! stop,
    {R1 - R0, R2 - R1}.

%% B has a message, so it is active, and it handles at most 100
%% reductions of signals at schedule-in; then it handles the rest in
%% loop_rec.
active(N) ->
    B = spawn(?MODULE, two, []),
    settle(),
    R0 = reds(B),
    B ! wake,
    repeat(N, fun() -> exit(B, normal) end),
    settle(),
    R1 = reds(B),
    B ! stop,
    R1 - R0.

%% B waits, so it handles the signals at schedule-in with its budget.
waiting(N) ->
    B = spawn(?MODULE, two, []),
    settle(),
    R0 = reds(B),
    repeat(N, fun() -> exit(B, normal) end),
    settle(),
    R1 = reds(B),
    B ! stop,
    R1 - R0.
