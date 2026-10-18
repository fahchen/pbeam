-module(bif_cost_probe).
-export([costs/1, longs/1, puts/1, put_trap/0, erase_costs/1, erase_trap/0, put_race/0, exits/0, bad_long/0,
         bad_binary/0, bad_latin1/0, results/0, binary_results/0, lows/1, gcs/1]).
-export([measure/3, measure_low/3, put_loop/2, erase_loop/2, erase_cost/1, erase_keys/0, exiter/2, watcher/3, traced/3, start/4,
         long_loop/5, send_loop/3]).

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
     %% pairs, and an atom. {dups, 40000} traps. {big_dups, 33} has one
     %% bignum key, which a PBeam hashmap does not support yet.
     %% {two_keys, A, B} has A pairs {1, 1} and then B pairs {2, 2}. The
     %% sort pivots one of the keys and then sorts the pairs of the other
     %% key: 73 pairs with a partition of 72 steps, or 9 pairs with the
     %% selection sort of 9 * 4 * 2 = 72 units (utils.c:3499-3588,
     %% 3636-3640). Thus the cost does not change with the pivot. PBeam
     %% pivots index 3 of 82, so it sorts the 73 pairs in the first input
     %% and the 9 pairs in the second.
     {maps_from_list,
      [nil, {flat, 32}, {dups, 33}, {dups, 80}, {dups, 400}, {dups, 40000},
       {big_dups, 33}, {two_keys, 9, 73}, {two_keys, 73, 9}, {bad, 400},
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
       {list, 50000}, {bin, 50000}]},
     %% A binary of bytes below 128 returns itself. {high_bin, 50000} and
     %% {list, 50000} trap.
     {characters_to_binary_latin1,
      [{list, 3}, {list, 100}, {list, 10000}, {list, 50000}, {bin, 3}, {bin, 10000},
       {high_bin, 3}, {high_bin, 10000}, {high_bin, 50000}, {one_bin, 10000}, {bins, 100},
       nested]},
     %% Iodata: see iodata/1. {list, 50000} traps in the size walk. The size
     %% walk of {list, 31970} uses all 3997 reductions, so the copy starts in
     %% the next turn. The copy of {big_bin, 1100000} and of
     %% {big_tail, 1100000} traps in the binary.
     {list_to_binary,
      [nil, {one_bin, 100}, {list, 3}, {list, 100}, {list, 10000}, {list, 31970},
       {list, 50000},
       {bins, 100}, nested, {big_bin, 1100000}, {big_tail, 1100000}, {bad_after, 400},
       {improper, 400}, atom]}].

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant.
id(X) -> X.

%% The pairs {K, K} for K in First..Last. Each '++' joins two halves. The
%% probe has this form from the time when PBeam copied a list at each cons
%% (before #458).
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
arg(maps_from_list, {dups, Count}) -> copies(Count, {1, 1});
arg(maps_from_list, {big_dups, Count}) -> copies(Count, {id(1 bsl 30) * (1 bsl 30), ok});
arg(maps_from_list, {two_keys, First, Second}) ->
    copies(First, {1, 1}) ++ copies(Second, {2, 2});
arg(maps_from_list, {bad, Count}) -> copies(Count, {1, 1}) ++ [x];
arg(maps_from_list, {improper, Count}) -> copies(Count, {1, 1}) ++ x;
arg(maps_from_list, atom) -> id(x);
arg(maps_keys, {_, Count}) -> maps:from_list(pairs(1, Count));
arg(characters_to_binary, Input) -> chardata(Input);
arg(characters_to_list, Input) -> chardata(Input);
arg(characters_to_list_latin1, Input) -> chardata(Input);
arg(characters_to_binary_latin1, Input) -> chardata(Input);
arg(list_to_binary, Input) -> iodata(Input);
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
run(characters_to_binary_latin1, Chardata) -> [unicode:characters_to_binary(Chardata, latin1)];
run(list_to_binary, Iodata) ->
    [try list_to_binary(Iodata)
     catch error:Reason -> Reason
     end];
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

%% The iodata of erlang:list_to_binary/1. {big_bin, N} has a byte and then
%% a binary of N bytes, and {big_tail, N} has the binary as its tail.
%% {bad_after, N} has an element that is not a byte after N bytes, and
%% {improper, N} the tail x.
iodata(nil) -> [];
iodata({big_bin, Count}) -> [$a, big_bytes(Count)];
iodata({big_tail, Count}) -> [$a | big_bytes(Count)];
iodata({bad_after, Count}) -> copies(Count, $a) ++ [256];
iodata({improper, Count}) -> copies(Count, $a) ++ x;
iodata(atom) -> id(x);
iodata(Input) -> chardata(Input).

%% A binary of Count bytes, a multiple of 1000, from binaries of 1000
%% bytes.
big_bytes(Count) ->
    list_to_binary(copies(Count div 1000, bytes(1000, $b))).

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

%% The results of unicode:characters_to_binary/2 with latin1,
%% unicode:bin_is_7bit/1, erlang:list_to_binary/1, and
%% erlang:split_binary/2 for each input shape, with each binary as
%% {binary, Bytes}, so that ~w writes the bytes and not a string.
binary_results() ->
    Latin1 = [unicode:characters_to_binary(Chardata, latin1)
              || Chardata <- [<<"abc">>, <<200, 65>>, [<<200>>, 65, [255]], [<<"a">>],
                              [<<200>>], [], [65 | <<200>>], chardata(nested)]],
    Latin1Errors = [catch_error(fun() -> unicode:characters_to_binary(Chardata, latin1) end)
                    || Chardata <- [[1, foo], foo, [a | b], [65 | x]]],
    Bit7 = [unicode:bin_is_7bit(Term)
            || Term <- [<<>>, <<"abc">>, <<128>>, <<1, 255>>, foo, [1], 1]],
    Lists = [catch_error(fun() -> list_to_binary(Iodata) end)
             || Iodata <- [[], [<<1>>], [1, [2, <<3>>] | <<4>>], [[]], [<<>>, []],
                           [<<1>>, <<2>>], [<<1>> | <<2>>], <<1>>, [256], [-1], foo, [a],
                           [1 | 2], [[1 | x]]]],
    Splits = [catch_error(fun() -> split_binary(Binary, Position) end)
              || {Binary, Position} <- [{<<"abc">>, 0}, {<<"abc">>, 1}, {<<"abc">>, 3},
                                        {<<>>, 0}, {<<"abc">>, 4}, {<<"abc">>, -1},
                                        {<<"abc">>, a}, {"abc", 1}]],
    show_binaries([Latin1, Latin1Errors, Bit7, Lists, Splits]).

catch_error(Fun) ->
    try Fun()
    catch error:Reason -> {error, Reason}
    end.

show_binaries(Binary) when is_binary(Binary) ->
    {binary, unicode:characters_to_list(Binary, latin1)};
show_binaries([Head | Tail]) ->
    [show_binaries(Head) | show_binaries(Tail)];
show_binaries({First, Second}) ->
    {show_binaries(First), show_binaries(Second)};
show_binaries(Term) ->
    Term.

%% unicode:characters_to_binary/2 with latin1 of a code point above 255.
%% ERTS returns {error, <<>>, [256]}, and PBeam returns a runtime error (row
%% "Chardata conversion" in docs/erts-differences.md).
bad_latin1() ->
    unicode:characters_to_binary([256], latin1).

%% For each K, the reductions of erlang:list_to_binary/1 of {list, 10000}
%% after the process uses K reductions: the call runs with 3996 - K
%% reductions left. With 100 or fewer, it switches out and runs again in
%% the next turn. With fewer than 1000, it walks with a budget of 1000
%% (erl_iolist.c:34-35, 866-876). The result includes K.
lows(Opts) ->
    Arg = iodata({list, 10000}),
    [{K, low_delta(K, Arg, Opts)} || K <- low_budgets()].

low_budgets() -> [3895, 3896, 3995, 2996, 2997, 0].

low_delta(K, Arg, Opts) ->
    Pid = spawn_opt(?MODULE, measure_low, [self(), K, Arg], Opts),
    receive {Pid, Delta} -> Delta end.

measure_low(Collector, K, Arg) ->
    erlang:yield(),
    Before = ?REDS,
    erlang:bump_reductions(K),
    run(list_to_binary, Arg),
    Collector ! {self(), ?REDS - Before}.

%% For each BIF and input, the reductions of the BIF: the reductions of
%% run/2 in a new process, without the reductions of run(none, _) and of
%% the call instruction.
costs(Opts) ->
    Base = delta(none, none, Opts),
    [{Bif, [{Input, delta(Bif, Input, Opts) - Base - 1} || Input <- Inputs]}
     || {Bif, Inputs} <- inputs()].

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
     {characters_to_list, {list, 50000}}, {characters_to_binary_latin1, {high_bin, 50000}},
     {list_to_binary, {list, 100000}}].

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

%% The reductions of persistent_term:erase/1 for the keys 1 to 10 in an
%% empty table, in the order of erase_keys/0. The table
%% grows to 32 slots at the ninth key, and an erase copies 96 slots when
%% it rehashes the same size, 80 slots when it shrinks to 16 slots, and no
%% slot when the next slot of the key is free (erl_bif_persistent.c:478-539,
%% 1025-1030). The erase switches out the caller.
erase_costs(Opts) ->
    Pid = spawn_opt(?MODULE, erase_loop, [self(), 10], Opts),
    receive {Pid, Costs} -> Costs end.

erase_loop(Collector, Count) ->
    [persistent_term:put(Key, Key) || Key <- keys(1, Count)],
    Costs = [erase_cost(Key) || Key <- erase_keys()],
    Collector ! {self(), Costs}.

erase_keys() -> [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].

%% The cost of one erase, without the call_ext of the light BIF and the
%% process_info/2 after it. The yield finishes the thread progress of the
%% updates of the earlier calls, so the erase does not wait for it and
%% uses no reductions for the wait.
erase_cost(Key) ->
    erlang:yield(),
    Before = ?REDS,
    persistent_term:erase(Key),
    ?REDS - Before - 2.

%% The reductions of an erase that rehashes the 16 slots of a table of 8
%% keys with 1 reduction left. The copies go through 48 slots, more than
%% the 32 of the budget. The erase uses all reductions and traps, and the
%% copies end in the next turn (erl_bif_persistent.c:275-293).
erase_trap() ->
    [persistent_term:put(Key, Key) || Key <- keys(1, 8)],
    erlang:yield(),
    Before = ?REDS,
    erlang:bump_reductions(3996),
    persistent_term:erase(1),
    ?REDS - Before.

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
     || {Bif, Inputs} <- [{none, [none]} | inputs()],
        Input <- Inputs]
    ++ [{puts, gc_count(put_loop, [70], Opts)}]
    ++ [{low, K, gc_count(measure_low, [K, iodata({list, 10000})], Opts)}
        || K <- low_budgets()]
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
