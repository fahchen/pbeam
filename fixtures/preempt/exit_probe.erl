-module(exit_probe).
-export([exits/2, gcs/2, late_timer/0]).
-export([exiter/5, watcher/3, relay/1, idle/0, late_exiter/1, late_starter/2]).

%% The reductions of the later phases of a process exit on the pinned ERTS
%% JIT: each BIF timer, link, monitor, and pending signal of the exiting
%% process uses reductions, and the exit switches out when they use the
%% budget (erts_continue_exit_process, erl_process.c:14270-14833).
%%
%% Each case {Setup, N, K} races a process a, which gets N items of the
%% kind Setup, and then exits after it uses all but 3997 - K reductions of
%% a turn, with a process b, which a spawns in that turn. b sends whether
%% a still has its name, and then 2000 tags. The collector returns the
%% messages that it gets in arrival order, as runs {Kind, Count}. With one
%% scheduler, the runs show the turn in which the exit of a sends each of
%% its signals.
%%
%% Opts are the spawn options of a and b. The ERTS reference run gives them
%% a large heap, so that no garbage collection occurs. PBeam has no process
%% heap and gets [].

exits(Cases, Opts) ->
    [{Case, race(Case, Opts, false)} || Case <- Cases].

%% ERTS only: the number of garbage collections of a and b in each case.
%% Each must give 0, else a collection changes the reductions on ERTS.
gcs(Cases, Opts) ->
    [{Case, gc_count(race(Case, Opts, true))} || Case <- Cases].

gc_count(Runs) ->
    lists:sum([Count || {gc, Count} <- Runs]).

race({Setup, N, K}, Opts, Trace) ->
    Root = self(),
    {A, _} = spawn_opt(?MODULE, exiter, [Root, Setup, N, K, Opts], [monitor | Opts]),
    Trace andalso erlang:trace(A, true, [garbage_collection, set_on_spawn, {tracer, Root}]),
    Downs = 1 + monitors(Setup, N, A),
    Exits = case Setup of links -> N; _ -> 0 end,
    collect(Downs, Exits, false, []).

%% The collector monitors a N more times for the cases watchers and
%% signals. a handles the monitor signals of watchers before its exit
%% turn, and many monitor signals of signals are still in its queue at its
%% exit.
monitors(Setup, N, A) when Setup =:= watchers; Setup =:= signals ->
    monitor_times(N, A);
monitors(_, _, _) ->
    0.

monitor_times(0, _) -> 0;
monitor_times(N, A) ->
    monitor(process, A),
    1 + monitor_times(N - 1, A).

%% a registers its name, makes its items, and switches out. In its next
%% turn it spawns b and exits: spawn_opt/4 1, bump_reductions/1 1 + K,
%% return 1.
exiter(Root, Setup, N, K, Opts) ->
    register(exiter, self()),
    items(Setup, N, Root),
    erlang:yield(),
    erlang:yield(),
    spawn_opt(?MODULE, watcher, [Root, exiter, 2000], Opts),
    erlang:bump_reductions(K),
    ok.

%% timers: N BIF timers to a. links: N linked processes that trap exits
%% and send their EXIT message to the collector. monitors: N monitors that
%% a holds.
items(timers, N, _) ->
    times(N, fun() -> erlang:start_timer(100000000, self(), tick) end);
items(links, N, Root) ->
    times(N, fun() -> spawn_link(?MODULE, relay, [Root]) end);
items(monitors, N, _) ->
    times(N, fun() -> monitor(process, spawn(?MODULE, idle, [])) end);
items(_, _, _) ->
    ok.

times(0, _) -> ok;
times(N, Fun) ->
    Fun(),
    times(N - 1, Fun).

relay(Root) ->
    process_flag(trap_exit, true),
    receive {'EXIT', _, Reason} -> Root ! {exit, Reason} end.

idle() ->
    receive stop -> ok end.

watcher(Collector, Name, Count) ->
    Collector ! {seen, whereis(Name) =/= undefined},
    tags(Collector, Count).

tags(Collector, 0) -> Collector ! done;
tags(Collector, Count) ->
    Collector ! b,
    tags(Collector, Count - 1).

%% Collect until b is done, each monitor of a gave its DOWN message, and
%% each relay sent its EXIT message.
collect(0, 0, true, Acc) ->
    lists:reverse(Acc, []);
collect(Downs, Exits, Done, Acc) ->
    receive
        b -> collect(Downs, Exits, Done, event(b, Acc));
        {seen, Seen} -> collect(Downs, Exits, Done, event({seen, Seen}, Acc));
        {'DOWN', _, process, _, Reason} ->
            collect(Downs - 1, Exits, Done, event({down, Reason}, Acc));
        {exit, Reason} -> collect(Downs, Exits - 1, Done, event({exit, Reason}, Acc));
        done -> collect(Downs, Exits, true, Acc);
        {trace, _, gc_minor_start, _} -> collect(Downs, Exits, Done, event(gc, Acc));
        {trace, _, _, _} -> collect(Downs, Exits, Done, Acc)
    end.

event(Kind, [{Kind, Count} | Acc]) -> [{Kind, Count + 1} | Acc];
event(Kind, Acc) -> [{Kind, 1} | Acc].

%% A BIF timer to a process whose exit switched out in its timer phase. The
%% process a starts 100 timers, so its exit switches out in that phase. b,
%% which a spawns in its exit turn, then starts a timer to a. ERTS gives no
%% timer to an exiting process (erl_hl_timer.c:1833-1861), so
%% cancel_timer/2 gives false.
late_timer() ->
    {A, Monitor} = spawn_opt(?MODULE, late_exiter, [self()], [monitor]),
    receive {late, Result} -> ok end,
    receive {'DOWN', Monitor, process, A, normal} -> Result end.

late_exiter(Root) ->
    times(100, fun() -> erlang:start_timer(100000000, self(), tick) end),
    erlang:yield(),
    spawn(?MODULE, late_starter, [Root, self()]),
    ok.

late_starter(Root, A) ->
    Ref = erlang:start_timer(100000000, A, late),
    Root ! {late, erlang:cancel_timer(Ref, [])}.
