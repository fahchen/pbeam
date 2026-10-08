-module(list_probe).
-export([costs/1, longs/1, gcs/1, start/4, traced/3]).
-export([cost_loop/5, long_loop/5, send_loop/3]).

%% The reductions and the traps of the list BIFs on the pinned ERTS JIT.
%% Each BIF reads the budget of the process (ERTS_BIF_REDS_LEFT), works on
%% one chunk of the list, and then bumps its cost, or uses all reductions
%% and traps to its export. The process then switches out, and the BIF
%% continues in its next turn.
%%
%% Opts are the spawn options of the measured processes. The ERTS reference
%% run gives them a large heap, so that lists:reverse/2 and erlang:'++'/2
%% build their result on the heap and no garbage collection occurs. PBeam
%% has no process heap and gets [].

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant.
id(X) -> X.

%% A list of N ones. Each '++' doubles the list, so that PBeam, which
%% copies a list at each cons, builds a long list in O(N log N).
ones(0) -> [];
ones(N) when N rem 2 =:= 0 ->
    Half = ones(N div 2),
    Half ++ Half;
ones(N) -> [1 | ones(N - 1)].

%% For each BIF and each list length N, two processes a and b run the same
%% loop. Each iteration sends the tag of the process to the collector and
%% calls the BIF on a list of N elements. The collector returns the run
%% lengths of the tags. With one scheduler, each run is one turn of a
%% process, so the run lengths show the cost of the BIF for N.
costs(Opts) ->
    [{Bif, [{N, race(cost_loop, [Bif, ones(N)], iterations(N), Opts)}
            || N <- [0, 1, 10, 100, 1000, 10000]]}
     || Bif <- [member, reverse, append, append_nil, length, keyfind, keymember]].

%% Enough iterations for more than two turns of each process.
iterations(N) when N =< 10 -> 1500;
iterations(100) -> 1000;
iterations(1000) -> 300;
iterations(10000) -> 30.

%% Process a calls a BIF on a long list three times, and sends its tag
%% after each call. Process b sends its tag 5000 times. Each long call
%% traps to its export at the end of the budget, so b runs between the
%% chunks of a.
longs(Opts) ->
    Long = ones(100000),
    [{Loop, long_race(Loop, long_list(Loop, Long), Opts)} || Loop <- long_loops()].

long_loops() ->
    [length_guard, length_body, length_apply, reverse_ext, reverse_apply_only,
     member_ext, keyfind_ext, keymember_ext].

%% lists:keymember/3 finds a tuple at the end of the list, so that its
%% result shows that each trap continues lists:keymember/3, not
%% lists:keyfind/3.
long_list(keymember_ext, Long) -> Long ++ [{x}];
long_list(_, Long) -> Long.

%% Start a and b from a new process, so that the reductions of the caller
%% do not change the order of the two spawns.
race(Loop, Args, N, Opts) ->
    spawn(?MODULE, start, [self(), {Loop, Args ++ [N]}, {Loop, Args ++ [N]}, Opts]),
    receive First -> collect(2 * N - 1, First, 1) end.

long_race(Loop, Long, Opts) ->
    spawn(?MODULE, start, [self(), {long_loop, [Loop, Long, 3]}, {send_loop, [5000]}, Opts]),
    receive First -> collect(5002, First, 1) end.

start(Collector, {LoopA, ArgsA}, {LoopB, ArgsB}, Opts) ->
    spawn_opt(?MODULE, LoopA, [Collector, a | ArgsA], Opts),
    spawn_opt(?MODULE, LoopB, [Collector, b | ArgsB], Opts).

%% Receive K more tags. Tag is the tag of the current run, and Count is its
%% length so far.
collect(0, Tag, Count) -> [{Tag, Count}];
collect(K, Tag, Count) ->
    receive
        Next when Next =:= Tag -> collect(K - 1, Tag, Count + 1);
        Next -> [{Tag, Count} | collect(K - 1, Next, 1)]
    end.

%% Entry 1, send 1 + 4, then the BIF: call_ext 1 and its cost, or the
%% length/1 guard BIF with no call cost.
cost_loop(P, T, Bif, L, N) ->
    cost_iterate(P, T, Bif, L, id([]), N).

cost_iterate(_, _, _, _, _, 0) -> ok;
cost_iterate(P, T, Bif, L, Nil, N) ->
    P ! T,
    case Bif of
        member -> false = lists:member(x, L);
        reverse -> _ = lists:reverse(L, Nil);
        append -> _ = L ++ [x];
        append_nil -> _ = L ++ Nil;
        length -> true = length(L) >= 0;
        keyfind -> false = lists:keyfind(x, 1, L);
        keymember -> false = lists:keymember(x, 1, L)
    end,
    cost_iterate(P, T, Bif, L, Nil, N - 1).

long_loop(_, _, _, _, 0) -> ok;
long_loop(P, T, Loop, L, N) ->
    long_call(Loop, L),
    P ! T,
    long_loop(P, T, Loop, L, N - 1).

%% length/1 in a guard and in a body keeps its trap state and runs the
%% instruction again. length/1 through apply/3 traps to erlang:length/3.
long_call(length_guard, L) when length(L) > 0 -> ok;
long_call(length_body, L) ->
    true = length(L) > 0,
    ok;
long_call(length_apply, L) ->
    apply(id(erlang), id(length), [L]),
    ok;
long_call(reverse_ext, L) ->
    _ = lists:reverse(L, []),
    ok;
long_call(reverse_apply_only, L) ->
    [1 | _] = reverse_only(L),
    ok;
long_call(member_ext, L) ->
    false = lists:member(x, L),
    ok;
long_call(keyfind_ext, L) ->
    false = lists:keyfind(x, 1, L),
    ok;
long_call(keymember_ext, L) ->
    true = lists:keymember(x, 1, L),
    ok.

%% apply_last of lists:reverse/2: the BIF returns from this frame when it
%% ends after a trap.
reverse_only(L) -> apply(id(lists), id(reverse), [L, []]).

send_loop(_, _, 0) -> ok;
send_loop(P, T, N) ->
    P ! T,
    send_loop(P, T, N - 1).

%% ERTS only: the number of garbage collections of each loop of a. Each
%% loop must give 0, else a collection changes the reductions on ERTS.
gcs(Opts) ->
    Long = ones(100000),
    [{Bif, N, gc_count(cost_loop, [Bif, ones(N), iterations(N)], Opts)}
     || Bif <- [member, reverse, append, append_nil, length, keyfind, keymember],
        N <- [0, 1, 10, 100, 1000, 10000]]
    ++ [{Loop, gc_count(long_loop, [Loop, long_list(Loop, Long), 3], Opts)}
        || Loop <- long_loops()].

gc_count(Loop, Args, Opts) ->
    Self = self(),
    {Pid, Ref} = spawn_opt(?MODULE, traced, [Self, Loop, Args], [monitor | Opts]),
    erlang:trace(Pid, true, [garbage_collection, {tracer, Self}]),
    Pid ! go,
    gc_events(Ref, 0).

traced(Collector, Loop, Args) ->
    receive go -> ok end,
    apply(?MODULE, Loop, [Collector, a | Args]).

gc_events(Ref, Count) ->
    receive
        {'DOWN', Ref, _, _, normal} -> Count;
        {trace, _, gc_minor_start, _} -> gc_events(Ref, Count + 1);
        {trace, _, gc_major_start, _} -> gc_events(Ref, Count + 1);
        _ -> gc_events(Ref, Count)
    end.
