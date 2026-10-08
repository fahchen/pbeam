-module(bif_cost_probe).
-export([costs/1, sorts/1, longs/1, puts/1, put_trap/0, put_race/0, exits/0, bad_long/0,
         bad_binary/0, results/0, gcs/1]).
-export([measure/3, put_loop/2, exiter/2, watcher/3, traced/3, start/4, long_loop/5,
         send_loop/3]).

%% The reductions of the BIFs whose ERTS cost depends on their input and
%% that #234 did not charge, and the reductions of a process exit, on the
%% pinned ERTS JIT.
%%
%% Opts are the spawn options of the measured processes. The ERTS reference
%% run gives them a large heap, so that no garbage collection occurs. PBeam
%% has no process heap and gets [].

%% The reduction total of the calling process. element/2 is a guard BIF,
%% so it costs no reduction.
-define(REDS, element(2, process_info(self(), reductions))).

%% The inputs of each BIF for costs/1. Each cost is the same in each ERTS
%% run.
inputs() ->
    [%% [], lists of {1, 1} (one key, so the sort has no pivot to pick),
     %% 32 keys (a flatmap), a bad element or an improper tail after N
     %% pairs, and an atom. {dups, 40000} traps.
     {maps_from_list,
      [nil, {flat, 32}, {dups, 33}, {dups, 80}, {dups, 400}, {dups, 40000}, {bad, 400},
       {improper, 400}, atom]},
     %% A map of the keys 1..N. {hash, 100000} traps.
     {maps_keys, [{flat, 32}, {hash, 33}, {hash, 100}, {hash, 10000}, {hash, 100000}]},
     %% Chardata: see chardata/1. Lists of 50000 characters, binaries of
     %% 50000 bytes, and a long binary after a list trap.
     {characters_to_binary,
      [{list, 3}, {list, 10}, {list, 49}, {list, 50}, {list, 100}, {list, 10000},
       {list, 50000}, {bin, 3}, {bin, 10000}, {one_bin, 10000}, {bins, 100}, nested,
       {list_bin, 50000}, {split, 39990}, {split_bin, 39990}, leftover_trap]},
     {characters_to_list,
      [{list, 3}, {list, 10}, {list, 100}, {list, 10000}, {list, 50000}, {bin, 3},
       {bin, 10000}, {bin, 39999}, {bin, 40000}, {bin, 50000}, {bin, 100000},
       {one_bin, 10000}, {bins, 100}, nested, {list_bin, 50000}, bad_code, bad_bin,
       short_bin, short_list, {bad_after, 50000}, leftover_trap]},
     {characters_to_list_latin1,
      [{list, 3}, {list, 100}, {list, 10000}, {bin, 3}, {bin, 10000}, {high_bin, 10000},
       {list, 50000}, {bin, 50000}]}].

%% The inputs of maps:from_list/1 whose ERTS cost changes between runs:
%% the pivots of the sort come from addresses (utils.c:3522-3526).
sort_inputs() ->
    [{distinct, 40}, {distinct, 100}, {distinct, 1000}, {distinct, 10000}].

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant.
id(X) -> X.

%% The pairs {K, K} for K in First..Last. Each '++' joins two halves, so
%% that PBeam, which copies a list at each cons, builds a long list in
%% O(N log N).
pairs(First, Last) when First > Last -> [];
pairs(Key, Key) -> [{Key, Key}];
pairs(First, Last) ->
    Middle = (First + Last) div 2,
    pairs(First, Middle) ++ pairs(Middle + 1, Last).

%% A list of Count copies of Term.
copies(0, _) -> [];
copies(1, Term) -> [Term];
copies(Count, Term) ->
    Half = Count div 2,
    copies(Half, Term) ++ copies(Count - Half, Term).

%% The argument of a BIF for an input. A process builds it before it
%% measures the BIF.
arg(maps_from_list, nil) -> [];
arg(maps_from_list, {flat, Count}) -> pairs(1, Count);
arg(maps_from_list, {distinct, Count}) -> pairs(1, Count);
arg(maps_from_list, {dups, Count}) -> copies(Count, {1, 1});
arg(maps_from_list, {bad, Count}) -> copies(Count, {1, 1}) ++ [x];
arg(maps_from_list, {improper, Count}) -> copies(Count, {1, 1}) ++ x;
arg(maps_from_list, atom) -> id(x);
arg(maps_keys, {_, Count}) -> maps:from_list(pairs(1, Count));
arg(characters_to_binary, Input) -> chardata(Input);
arg(characters_to_list, Input) -> chardata(Input);
arg(characters_to_list_latin1, Input) -> chardata(Input);
arg(persistent_term_put, Key) -> Key;
arg(none, _) -> none.

%% Call a BIF with its argument. The list keeps the result, so the
%% compiler keeps the call, and the call is a body call_ext of a light
%% BIF, which costs 1.
run(maps_from_list, List) ->
    [try maps:from_list(List)
     catch error:Reason -> Reason
     end];
run(maps_keys, Map) -> [maps:keys(Map)];
run(characters_to_binary, Chardata) -> [unicode:characters_to_binary(Chardata, unicode)];
run(characters_to_list, Chardata) -> [unicode:characters_to_list(Chardata, unicode)];
run(characters_to_list_latin1, Chardata) -> [unicode:characters_to_list(Chardata, latin1)];
run(persistent_term_put, Key) -> [persistent_term:put(Key, make_ref())];
run(none, _) -> [].

%% The chardata of the unicode BIFs. {list, N} has N times $a, {bin, N}
%% is a binary of N bytes, {one_bin, N} a list of one such binary, and
%% {bins, N} a list of N binaries of 100 bytes. {list_bin, N} has N
%% characters and then a binary of 100 bytes. {split, N} has N characters
%% and a binary of a two-byte character, and {split_bin, N} a binary of N
%% bytes and then that binary. bad_code, bad_bin, short_bin, and short_list
%% give an error or incomplete result, and {bad_after, N} has an invalid
%% code point after N characters.
chardata({list, Count}) -> copies(Count, $a);
chardata({bin, Count}) -> bytes(Count, $a);
chardata({high_bin, Count}) -> bytes(Count, 233);
chardata({one_bin, Count}) -> [bytes(Count, $a)];
chardata({bins, Count}) -> copies(Count, bytes(100, $b));
chardata(nested) -> [[$a, [$b, <<"cd">>], []], [[[$e]]], <<"f">> | <<"g">>];
chardata({list_bin, Count}) -> [copies(Count, $a), bytes(100, $b)];
chardata({split, Count}) -> [copies(Count, $a), <<"é"/utf8>>];
chardata({split_bin, Count}) -> [bytes(Count, $a), <<"é"/utf8>>];
chardata(bad_code) -> [$a, [$b, -1], $c];
chardata(bad_bin) -> [<<97, 255, 98>>, $c];
chardata(short_bin) -> <<97, 226, 130>>;
chardata(short_list) -> [$a, <<226>>, [<<130>>]];
chardata({bad_after, Count}) -> copies(Count, $a) ++ [-1];
%% The steps end after <<226>>, which starts a sequence that the next
%% binary completes: the conversion traps with the bytes of the sequence
%% before the rest.
chardata(leftover_trap) -> [copies(28547, $a), <<"bb">>, <<226>>, <<130, 172>>].

%% A binary of Count copies of Byte. erlang:iolist_to_iovec/1 puts the
%% bytes of a list into one binary.
bytes(Count, Byte) ->
    [Binary] = erlang:iolist_to_iovec(copies(Count, Byte)),
    Binary.

%% The results of unicode:characters_to_list/2 for the chardata that
%% gives an error or an incomplete result, and for the nested chardata.
results() ->
    [unicode:characters_to_list(chardata(Input), Encoding)
     || Input <- [nested, bad_code, bad_bin, short_bin, short_list, {bad_after, 3}],
        Encoding <- [unicode, latin1]].

%% For each BIF and input, the reductions of the BIF: the reductions of
%% run/2 in a new process, without the reductions of run(none, _) and of
%% the call instruction.
costs(Opts) ->
    Base = delta(none, none, Opts),
    [{Bif, [{Input, delta(Bif, Input, Opts) - Base - 1} || Input <- Inputs]}
     || {Bif, Inputs} <- inputs()].

%% The costs of maps:from_list/1 for the inputs whose ERTS cost changes
%% between runs.
sorts(Opts) ->
    Base = delta(none, none, Opts),
    [{Input, delta(maps_from_list, Input, Opts) - Base - 1} || Input <- sort_inputs()].

delta(Bif, Input, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), Bif, Input], Opts),
    receive {Pid, Delta} -> Delta end.

%% Build the argument, and switch out, so that the BIF runs with the budget
%% of a new turn: 4000, minus 1 for the first process_info/2, 1 for the
%% entry of run/2, and 1 for call_ext.
measure(Collector, Bif, Input) ->
    Arg = arg(Bif, Input),
    erlang:yield(),
    Before = ?REDS,
    run(Bif, Arg),
    Collector ! {self(), ?REDS - Before}.

%% maps:from_list/1 of a list whose check yields before a bad element.
%% ERTS loops forever: BIF_ERROR returns before YCF clears its trap state,
%% so the trap driver traps again (utils.c:3757-3773). PBeam returns a
%% runtime error.
bad_long() ->
    maps:from_list(copies(170000, {1, 1}) ++ [x]).

%% The inputs of the BIFs that trap, for longs/1.
long_inputs() ->
    [{maps_from_list, {dups, 40000}}, {maps_keys, {hash, 100000}},
     {characters_to_binary, {list, 50000}}, {characters_to_list, {bin, 100000}},
     {characters_to_list, {list, 50000}}].

%% For each BIF that traps, two processes a and b. Process a calls the BIF
%% three times and sends its tag after each call. Process b sends its tag
%% 5000 times. The collector returns the run lengths of the tags. With one
%% scheduler, each run is one turn of a process, so b runs between the
%% traps of a.
longs(Opts) ->
    [{Bif, long_race(Bif, Input, Opts)} || {Bif, Input} <- long_inputs()].

long_race(Bif, Input, Opts) ->
    Arg = arg(Bif, Input),
    spawn(?MODULE, start, [self(), {long_loop, [Bif, Arg, 3]}, {send_loop, [5000]}, Opts]),
    receive First -> runs(5002, First, 1) end.

start(Collector, {LoopA, ArgsA}, {LoopB, ArgsB}, Opts) ->
    spawn_opt(?MODULE, LoopA, [Collector, a | ArgsA], Opts),
    spawn_opt(?MODULE, LoopB, [Collector, b | ArgsB], Opts).

%% Receive K more tags. Tag is the tag of the current run, and Count is its
%% length so far.
runs(0, Tag, Count) -> [{Tag, Count}];
runs(K, Tag, Count) ->
    receive
        Next when Next =:= Tag -> runs(K - 1, Tag, Count + 1);
        Next -> [{Tag, Count} | runs(K - 1, Next, 1)]
    end.

long_loop(_, _, _, _, 0) -> ok;
long_loop(P, T, Bif, Arg, N) ->
    run(Bif, Arg),
    P ! T,
    long_loop(P, T, Bif, Arg, N - 1).

send_loop(_, _, 0) -> ok;
send_loop(P, T, N) ->
    P ! T,
    send_loop(P, T, N - 1).

%% The reductions of a put that grows the table with 1 reduction left. The
%% table of 16 slots holds 8 keys, so the copy goes through 48 slots, more
%% than the 32 of the budget. The put uses all reductions and traps, and
%% the copy ends in the next turn (erl_bif_persistent.c:275-293). The
%% caller must erase the persistent terms first.
put_trap() ->
    [persistent_term:put({t, Key}, Key) || Key <- keys(1, 8)],
    erlang:yield(),
    Before = ?REDS,
    erlang:bump_reductions(3996),
    persistent_term:put({t, 9}, 9),
    ?REDS - Before.

%% unicode:characters_to_binary/2 of invalid UTF-8. ERTS returns
%% {error, <<>>, <<255>>}, and PBeam returns a runtime error (row
%% "Chardata conversion" in docs/erts-differences.md).
bad_binary() ->
    unicode:characters_to_binary([<<255>>], unicode).

%% The runs of a process a that puts a new value three times, and of b. On
%% PBeam, a switches out at each put. ERTS suspends a until thread progress
%% makes the new table visible, so b runs more turns (row "Persistent
%% terms" in docs/erts-differences.md).
put_race() ->
    long_race(persistent_term_put, race, []).

%% The reductions of persistent_term:put/2 for the keys {p, 1} to
%% {p, Count} in an empty table, then for the same value again and for a
%% new value. The table grows at 5, 9, 17, 33, and 65 keys, and the copy
%% costs one reduction for each 32 slots of the old and the new table
%% (erl_bif_persistent.c:46, 708-718, 1034-1139). The result keeps the
%% costs of the keys in put_keys/0: on ERTS, the caller sometimes uses 2
%% to 4 more reductions after it continues from the suspension of a put
%% (row "Persistent terms" in docs/erts-differences.md).
puts(Opts) ->
    Pid = spawn_opt(?MODULE, put_loop, [self(), 70], Opts),
    receive {Pid, Costs} -> Costs end.

put_keys() -> [1, 2, 4, 5, 8, 9, 10, 17, 33, 34, 65].

put_loop(Collector, Count) ->
    New = [put_cost({p, Key}, Key) || Key <- keys(1, Count)],
    Same = put_cost({p, 1}, 1),
    Changed = put_cost({p, 1}, 2),
    Kept = [{Key, nth(Key, New)} || Key <- put_keys()],
    Collector ! {self(), {Kept, Same, Changed}}.

nth(1, [Head | _]) -> Head;
nth(N, [_ | Tail]) -> nth(N - 1, Tail).

keys(First, Last) when First > Last -> [];
keys(First, Last) -> [First | keys(First + 1, Last)].

%% The cost of one put, without the call_ext of the light BIF and the
%% process_info/2 after it.
put_cost(Key, Value) ->
    Before = ?REDS,
    persistent_term:put(Key, Value),
    ?REDS - Before - 2.

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

%% ERTS only: the number of garbage collections of each measured process.
%% Each must give 0, else a collection changes the reductions on ERTS.
gcs(Opts) ->
    [{Bif, Input, gc_count(measure, [Bif, Input], Opts)}
     || {Bif, Inputs} <- [{none, [none]}, {maps_from_list, sort_inputs()} | inputs()],
        Input <- Inputs]
    ++ [{puts, gc_count(put_loop, [70], Opts)}]
    ++ [{Bif, gc_count(long_loop, [a, Bif, arg(Bif, Input), 3], Opts)}
        || {Bif, Input} <- long_inputs()].

gc_count(Function, Args, Opts) ->
    Self = self(),
    {Pid, Ref} = spawn_opt(?MODULE, traced, [Self, Function, Args], [monitor | Opts]),
    erlang:trace(Pid, true, [garbage_collection, {tracer, Self}]),
    Pid ! go,
    gc_events(Ref, 0).

traced(Collector, Function, Args) ->
    receive go -> ok end,
    apply(?MODULE, Function, [Collector | Args]).

gc_events(Ref, Count) ->
    receive
        {'DOWN', Ref, _, _, normal} -> Count;
        {trace, _, gc_minor_start, _} -> gc_events(Ref, Count + 1);
        {trace, _, gc_major_start, _} -> gc_events(Ref, Count + 1);
        _ -> gc_events(Ref, Count)
    end.
