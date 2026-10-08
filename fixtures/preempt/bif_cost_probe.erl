-module(bif_cost_probe).
-export([exits/0]).
-export([exiter/2, watcher/3]).

%% The reductions of a process exit on the pinned ERTS JIT.

%% For each budget K, a process a that exits after it uses all but
%% 3996 - K reductions, and a process b that sends whether a has its name
%% and then 1000 tags. The exit bumps 100 reductions (erl_process.c:14229).
%% With 100 or fewer reductions left, a switches out before it leaves the
%% process table, and b sees its name. With 150 or fewer, a switches out
%% before its links and monitors (erl_process.c:14315-14316, 14475,
%% 14541-14543). The DOWN message then comes after a turn of b.
exits() ->
    [{K, exit_race(K)} || K <- [3845, 3846, 3895, 3896, 3990]].

exit_race(K) ->
    {_, Ref} = spawn_opt(?MODULE, exiter, [exiter, K], [monitor]),
    spawn(?MODULE, watcher, [self(), exiter, 1000]),
    collect(Ref, 0, []).

%% Entry 1, register/2 1, bump_reductions/1 1 + K, return 1.
exiter(Name, K) ->
    register(Name, self()),
    erlang:bump_reductions(K),
    ok.

watcher(Collector, Name, Count) ->
    Collector ! {seen, whereis(Name) =/= undefined},
    tags(Collector, Count).

tags(Collector, 0) -> Collector ! done;
tags(Collector, Count) ->
    Collector ! b,
    tags(Collector, Count - 1).

collect(Ref, Count, Acc) ->
    receive
        b -> collect(Ref, Count + 1, Acc);
        {seen, Seen} -> collect(Ref, Count, [{seen, Seen} | Acc]);
        {'DOWN', Ref, _, _, Reason} -> collect(Ref, 0, [{down, Count, Reason} | Acc]);
        done -> reverse([{done, Count} | Acc], [])
    end.

reverse([], Acc) -> Acc;
reverse([Head | Tail], Acc) -> reverse(Tail, [Head | Acc]).
