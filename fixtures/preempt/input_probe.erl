-module(input_probe).
-export([costs/1, longs/1, gcs/1]).
-export([measure/3, start/4, long_loop/5, send_loop/3, traced/3, owner/2, vanish_insert/3,
         vanish_stop/1, idle/0]).

%% The reductions of the BIFs whose ERTS cost depends on their input, on
%% the pinned ERTS JIT. A BIF reads the budget of the process
%% (ERTS_BIF_REDS_LEFT) and bumps the cost of its work. A BIF with more
%% work than its budget uses all reductions and traps to its export, so the
%% process switches out, and the BIF continues in its next turn.
%%
%% Opts are the spawn options of the measured processes. The ERTS reference
%% run gives them a large heap, so that no garbage collection occurs. PBeam
%% has no process heap and gets [].

%% The reduction total of the calling process. element/2 is a guard BIF,
%% so it costs no reduction.
-define(REDS, element(2, process_info(self(), reductions))).

%% 100 bytes, as a string, so that a binary of copies of it is a literal.
-define(S100, "0123456789012345678901234567890123456789012345678901234567890123456789"
              "012345678901234567890123456789").

%% The inputs of each BIF for costs/1.
inputs() ->
    [%% A small integer, the largest and the smallest small integer
     %% (2^59 - 1 and -2^59), the bignums next to them, and the largest
     %% and the smallest 64-bit integer.
     {integer_to_list,
      [small, max_small, min_big, min_small, max_negative_big, max_64, min_64]},
     {integer_to_binary,
      [small, max_small, min_big, min_small, max_negative_big, max_64, min_64]},
     %% {Integer, Base}: the estimate of a bignum depends on the base.
     {integer_to_binary_2,
      [{small, 2}, {max_small, 2}, {min_big, 2}, {max_negative_big, 2}, {max_64, 2},
       {min_64, 2}, {max_64, 10}, {max_64, 36}]},
     %% Two maps of the keys First..Last: two flatmaps, an empty flatmap
     %% and a hashmap, a flatmap and a hashmap, and two hashmaps. The
     %% last pair traps.
     {maps_merge,
      [{{1, 10}, {5, 20}}, {{1, 0}, {1, 40}}, {{1, 10}, {100, 140}}, {{100, 140}, {1, 10}},
       {{1, 40}, {41, 80}}, {{1, 40}, {1, 40}}, {{1, 100}, {50, 150}},
       {{1, 1000}, {500, 1500}}, {{1, 10000}, {5000, 15000}},
       {{1, 40000}, {20000, 60000}}]},
     %% {Size, Path, Acc} for a map of the keys 1..Size, and
     %% {ordered, Size, Keys} for an ordered iterator path of the keys
     %% 1..Keys.
     {map_next,
      [{10, 0, []}, {33, 0, []}, {33, 1, iterator}, {100, 1, []}, {1000, 1, []},
       {10000, 1, iterator}, {ordered, 200, 1}, {ordered, 200, 124}, {ordered, 200, 125}]},
     %% One object, [], a list of one object, lists of N objects, an
     %% improper list of 100 objects, a list of 100 objects with an atom at
     %% position 50, lists for a table that the caller cannot write, and a
     %% list for a table that does not exist. A list of 16000 or more
     %% objects traps.
     {ets_insert,
      [object, nil, one, {list, 2}, {list, 3}, {list, 8}, {list, 100}, {list, 1000},
       {list, 10000}, {list, 15000}, {list, 16000}, improper, bad, {protected, 100},
       {protected, 40000}, {missing, 100}]},
     %% The number of persistent terms of the node. The numbers go up,
     %% because no BIF of PBeam erases a persistent term.
     {persistent_term_get, [0, 1, 100, 1000, 4000, 10000]},
     %% Iodata: see iodata/1. A walk of more than 31975 steps traps.
     {iolist_size,
      [bin, nil, {ones, 1}, {ones, 7}, {ones, 8}, {ones, 15}, {ones, 16}, nested, binaries,
       {ones, 1000}, {ones, 10000}, {ones, 31990}, {ones, 100000}, bad, bad_tail, atom,
       {bad_after, 40000}, {tail_bin, 15983}]},
     {iolist_to_iovec,
      [bin, nil, empty_bin_list, one_bin, {ones, 1}, {ones, 8}, {ones, 16}, nested, binaries,
       {ones, 1000}, {ones, 31970}, {ones, 31990}, {ones, 100000}, bad, bad_tail, atom,
       {bins, 1995}, {tail_bin, 15983}]},
     {iolist_to_binary,
      [bin, nil, empty_bin_list, one_bin, {ones, 1}, {ones, 8}, {ones, 16}, nested, binaries,
       {ones, 1000}, {ones, 31970}, {ones, 31990}, {ones, 100000}, bad, bad_tail, atom,
       {bins, 1995}, {tail_bin, 15983}]},
     %% {Bump, Name}: iolist_to_binary/1 after erlang:bump_reductions(Bump).
     %% With at most 100 reductions left, the call switches out first. With
     %% fewer than 1000, it walks with a budget of 1000.
     {iolist_to_binary_late,
      [{3900, {ones, 10}}, {3900, atom}, {3500, nested}, {3500, {ones, 4000}},
       {3500, {ones, 8000}}]},
     %% binary:split/3 inputs: see split_arg/1.
     {binary_split,
      [first, global, none, long_needle, ac_first, ac_global, trim, bad, {commas, 10},
       {commas, 1000}, {commas, 20000}, {commas, 50000}, {plain, 1000}, {plain, 100000},
       {plain, 400000}, {plain_global, 100000}, {plain_ac, 1000}, {plain_ac, 100000},
       {mixed, 1000}, {mixed, 30000}, {wrap, 319700}, {wrap, 319745}, {wrap, 319760}]},
     %% {Process, Item}: the caller, a process with no message, and a
     %% process with 10000 messages that it did not receive.
     {process_info,
      [{self, registered_name}, {self, message_queue_len}, {self, reductions},
       {self, current_stacktrace}, {idle, message_queue_len}, {idle, registered_name},
       {idle, reductions}, {busy, message_queue_len}]}].

%% The inputs of the BIFs that trap, for longs/1.
long_inputs() ->
    [{maps_merge, {{1, 40000}, {20000, 60000}}}, {ets_insert, {list, 16000}},
     {persistent_term_get, 10000}, {iolist_size, {ones, 100000}},
     {iolist_to_iovec, {ones, 100000}}, {iolist_to_iovec, {bins, 1995}},
     {iolist_to_binary, {ones, 100000}}, {iolist_to_binary, {bins, 1995}},
     {iolist_to_binary_late, {3900, {ones, 10}}}, {binary_split, {commas, 50000}},
     {binary_split, {plain_ac, 200000}}].

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant.
id(X) -> X.

%% The keys First..Last as pairs {K, K}. Each '++' joins two halves, so
%% that PBeam, which copies a list at each cons, builds a long list in
%% O(N log N).
pairs(First, Last) when First > Last -> [];
pairs(Key, Key) -> [{Key, Key}];
pairs(First, Last) ->
    Middle = (First + Last) div 2,
    pairs(First, Middle) ++ pairs(Middle + 1, Last).

%% The keys First..Last.
keys(First, Last) -> [Key || {Key, _} <- pairs(First, Last)].

map({First, Last}) -> maps:from_list(pairs(First, Last)).

%% The integers of integer_to_list/1 and integer_to_binary/1,2. A bignum
%% literal is not a PBeam literal, so the arithmetic builds each bignum.
integer(small) -> id(12345);
integer(max_small) -> id(576460752303423487);
integer(min_big) -> id(576460752303423487) + 1;
integer(min_small) -> id(-576460752303423488);
integer(max_negative_big) -> id(-576460752303423488) - 1;
integer(max_64) -> id(576460752303423487) * 16 + 15;
integer(min_64) -> id(-576460752303423488) * 16.

%% The argument of a BIF for an input. A process builds it before it
%% measures the BIF.
arg(integer_to_list, Name) -> integer(Name);
arg(integer_to_binary, Name) -> integer(Name);
arg(integer_to_binary_2, {Name, Base}) -> {integer(Name), Base};
arg(maps_merge, {First, Second}) -> {map(First), map(Second)};
arg(map_next, {ordered, Size, Keys}) -> {keys(1, Keys), map({1, Size}), iterator};
arg(map_next, {Size, Path, Acc}) -> {Path, map({1, Size}), Acc};
arg(ets_insert, Input) -> {table(Input), objects(Input)};
arg(persistent_term_get, _) -> none;
arg(iolist_size, Name) -> iodata(Name);
arg(iolist_to_iovec, Name) -> iodata(Name);
arg(iolist_to_binary, Name) -> iodata(Name);
arg(iolist_to_binary_late, {Bump, Name}) -> {Bump, iodata(Name)};
arg(binary_split, Name) -> split_arg(Name);
arg(process_info, {self, Item}) -> {self(), Item};
arg(process_info, {idle, Item}) -> {spawn(?MODULE, idle, []), Item};
arg(process_info, {busy, Item}) ->
    Pid = spawn(?MODULE, idle, []),
    send_n(Pid, 10000),
    {Pid, Item};
arg(none, _) -> none.

%% A public table of the caller, or a protected table of another process.
table({protected, _}) -> element(2, owned_table(protected));
table({missing, _}) -> missing;
table(_) -> ets:new(input, [public]).

owned_table(Access) ->
    Owner = spawn(?MODULE, owner, [self(), Access]),
    receive {table, Owner, Table} -> {Owner, Table} end.

%% Make a table, give it to the collector, and keep it until stop.
owner(Collector, Access) ->
    Collector ! {table, self(), ets:new(owned, [Access])},
    receive stop -> ok end.

objects(object) -> {1, 1};
objects(nil) -> [];
objects(one) -> [{1, 1}];
objects({_, Count}) -> pairs(1, Count);
objects(improper) -> pairs(1, 100) ++ x;
objects(bad) -> pairs(1, 49) ++ [x | pairs(51, 100)].

%% The iodata of the iolist BIFs. A cell and the end of each list use one
%% step of the walk, and the copy of iolist_to_iovec/1 uses one more step
%% for each 32 bytes of a binary of at most 512 bytes. {bins, N} has N
%% binaries of 500 bytes, and the copy stops in a binary. {tail_bin, N} has
%% N ones and a binary of 500 bytes as its tail, and the copy stops in the
%% tail.
iodata(bin) -> <<1, 2, 3>>;
iodata(nil) -> [];
iodata(empty_bin_list) -> [<<>>];
iodata(one_bin) -> [<<1, 2, 3>>];
iodata({ones, Count}) -> ones(Count);
iodata(nested) -> [[1, 2], [3], [], [[]], <<4>>];
iodata(binaries) ->
    [<<?S100 ?S100 ?S100 ?S100 ?S100 ?S100 ?S100 ?S100>>, <<?S100>>,
     <<?S100 ?S100 ?S100 ?S100 ?S100 ?S100>>];
iodata(bad) -> [1, 2, a];
iodata(bad_tail) -> [1, 2 | a];
iodata(atom) -> a;
iodata({bad_after, Count}) -> ones(Count - 1) ++ [a];
iodata({bins, Count}) -> copies(Count, <<?S100 ?S100 ?S100 ?S100 ?S100>>);
iodata({tail_bin, Count}) -> ones(Count) ++ <<?S100 ?S100 ?S100 ?S100 ?S100>>.

%% {Subject, Pattern, Options} of binary:split/3. One pattern is a
%% Boyer-Moore search, and more patterns are an Aho-Corasick search. The
%% long subjects are binaries of copies of a unit: {commas, N} has N
%% matches, {plain, N} has none, and {mixed, N} has 2 * N matches. The
%% search of {wrap, N} reads N bytes before the last byte of a needle of 19
%% bytes, so the steps of the memchr skip can wrap below 0.
split_arg(first) -> {<<"a,b,c">>, <<",">>, []};
split_arg(global) -> {<<"a,b,,c,">>, <<",">>, [global]};
split_arg(none) -> {<<"abc">>, <<",">>, []};
split_arg(long_needle) -> {<<"abcabdabcabxxabcab">>, <<"abcab">>, [global]};
split_arg(ac_first) -> {<<"banana">>, [<<"an">>, <<"nan">>], []};
split_arg(ac_global) -> {<<"banana">>, [<<"an">>, <<"nan">>, <<"a">>], [global]};
split_arg(trim) -> {<<",a,,b,,">>, <<",">>, [global, trim_all]};
split_arg(bad) -> {<<"a">>, <<>>, []};
split_arg({commas, Count}) -> {repeat(Count, <<"ab,">>), <<",">>, [global]};
split_arg({plain, Count}) -> {repeat(Count, <<"x">>), <<"y">>, []};
split_arg({plain_global, Count}) -> {repeat(Count, <<"x">>), <<"y">>, [global]};
split_arg({plain_ac, Count}) -> {repeat(Count, <<"x">>), [<<"ab">>, <<"y">>], []};
split_arg({mixed, Count}) -> {repeat(Count, <<"xyab">>), [<<"ab">>, <<"y">>], [global]};
split_arg({wrap, Count}) ->
    Needle = <<"0123456789abcdefghZ">>,
    {iolist_to_binary([repeat(Count, <<"x">>), Needle]), Needle, []}.

%% A binary of Count copies of Unit, built by doubling.
repeat(0, _) -> <<>>;
repeat(Count, Unit) ->
    Half = repeat(Count div 2, Unit),
    case Count rem 2 of
        0 -> <<Half/binary, Half/binary>>;
        1 -> <<Half/binary, Half/binary, Unit/binary>>
    end.

%% Wait for stop.
idle() ->
    receive stop -> ok end.

send_n(_, 0) -> ok;
send_n(Pid, Count) ->
    Pid ! x,
    send_n(Pid, Count - 1).

%% A list of Count ones, or of Count copies of Term. Each '++' joins two
%% halves.
ones(Count) -> copies(Count, 1).

copies(0, _) -> [];
copies(1, Term) -> [Term];
copies(Count, Term) ->
    Half = Count div 2,
    copies(Half, Term) ++ copies(Count - Half, Term).

%% Prepare the state of the node for an input. The caller puts the
%% persistent terms, so that the measured process collects no garbage.
prepare(persistent_term_get, Count) -> put_terms(Count);
prepare(_, _) -> ok.

%% Put the persistent terms {k, N} for N in 1..Count. A key that has the
%% same value already changes nothing.
put_terms(0) -> ok;
put_terms(Count) ->
    persistent_term:put({k, Count}, Count),
    put_terms(Count - 1).

%% Call a BIF with its argument. The list keeps the result, so the
%% compiler keeps the call, and the call is a body call_ext.
run(integer_to_list, Value) -> [integer_to_list(Value)];
run(integer_to_binary, Value) -> [integer_to_binary(Value)];
run(integer_to_binary_2, {Value, Base}) -> [integer_to_binary(Value, Base)];
run(maps_merge, {First, Second}) -> [maps:merge(First, Second)];
run(map_next, {Path, Map, Acc}) -> [erts_internal:map_next(Path, Map, Acc)];
run(ets_insert, {Table, Objects}) ->
    [try ets:insert(Table, Objects)
     catch error:Reason -> Reason
     end];
run(persistent_term_get, _) -> [persistent_term:get()];
run(iolist_size, Iodata) ->
    [try iolist_size(Iodata)
     catch error:Reason -> Reason
     end];
run(iolist_to_iovec, Iodata) ->
    [try erlang:iolist_to_iovec(Iodata)
     catch error:Reason -> Reason
     end];
run(iolist_to_binary, Iodata) ->
    [try iolist_to_binary(Iodata)
     catch error:Reason -> Reason
     end];
run(iolist_to_binary_late, {Bump, Iodata}) ->
    erlang:bump_reductions(Bump),
    [try iolist_to_binary(Iodata)
     catch error:Reason -> Reason
     end];
run(binary_split, {Subject, Pattern, Options}) ->
    [try binary:split(Subject, Pattern, Options)
     catch error:Reason -> Reason
     end];
run(process_info, {Pid, Item}) -> [process_info(Pid, Item)];
run(none, _) -> [].

%% The cost of the call instruction: call_ext of a light BIF costs 1, and
%% of the heavy BIF process_info/2 0.
call(process_info) -> 0;
call(_) -> 1.

%% For each BIF and input, the reductions of the BIF: the reductions of
%% run/2 in a new process, without the reductions of run(none, _) and of
%% the call instruction.
costs(Opts) ->
    Base = delta(none, none, Opts),
    [{Bif, [{Input, delta(Bif, Input, Opts) - Base - call(Bif)} || Input <- Inputs]}
     || {Bif, Inputs} <- inputs()].

delta(Bif, Input, Opts) ->
    prepare(Bif, Input),
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

%% For each BIF that traps, two processes a and b. Process a calls the BIF
%% three times and sends its tag after each call. Process b sends its tag
%% 5000 times. The collector returns the run lengths of the tags. With one
%% scheduler, each run is one turn of a process, so b runs between the
%% traps of a.
longs(Opts) ->
    [{Bif, long_race(Bif, Input, Opts)} || {Bif, Input} <- long_inputs()]
    ++ [{ets_vanish, vanish(Opts)}, {iovec_sizes, iovec_sizes()}].

%% The size of the iodata and of each binary of its iovec, for the inputs
%% whose copy traps or stops in a binary.
iovec_sizes() ->
    [{Name, iolist_size(iodata(Name)),
      [iolist_size(Binary) || Binary <- erlang:iolist_to_iovec(iodata(Name))]}
     || Name <- [binaries, {ones, 100000}, {bins, 1995}, {tail_bin, 15983}]].

%% The caller builds the argument, so that the reductions of the build do
%% not change the runs.
long_race(Bif, Input, Opts) ->
    prepare(Bif, Input),
    Arg = arg(Bif, Input),
    spawn(?MODULE, start, [self(), {long_loop, [Bif, Arg, 3]}, {send_loop, [5000]}, Opts]),
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

long_loop(_, _, _, _, 0) -> ok;
long_loop(P, T, Bif, Arg, N) ->
    run(Bif, Arg),
    P ! T,
    long_loop(P, T, Bif, Arg, N - 1).

%% A process inserts 16000 objects into the table of another process. In
%% its first turn, it checks the objects and starts to copy them. Another
%% process then sends stop to the owner. In its second turn, the insert
%% puts the objects into the table, and traps. The owner then exits, and
%% the insert raises badarg in its third turn.
vanish(Opts) ->
    {Owner, Table} = owned_table(public),
    spawn_opt(?MODULE, vanish_insert, [self(), Table, pairs(1, 16000)], Opts),
    spawn_opt(?MODULE, vanish_stop, [Owner], Opts),
    receive {inserted, Result} -> Result end.

vanish_insert(Collector, Table, Objects) ->
    Collector ! {inserted, run(ets_insert, {Table, Objects})}.

vanish_stop(Owner) -> Owner ! stop.

send_loop(_, _, 0) -> ok;
send_loop(P, T, N) ->
    P ! T,
    send_loop(P, T, N - 1).

%% ERTS only: the number of garbage collections of each measured process
%% and of each process a. Each must give 0, else a collection changes the
%% reductions on ERTS.
gcs(Opts) ->
    [{Bif, Input, gc_count(Bif, Input, measure, [Bif, Input], Opts)}
     || {Bif, Inputs} <- [{none, [none]} | inputs()], Input <- Inputs]
    ++ [{Bif, gc_count(Bif, Input, long_loop, [a, Bif, arg(Bif, Input), 3], Opts)}
        || {Bif, Input} <- long_inputs()].

gc_count(Bif, Input, Loop, Args, Opts) ->
    prepare(Bif, Input),
    Self = self(),
    {Pid, Ref} = spawn_opt(?MODULE, traced, [Self, Loop, Args], [monitor | Opts]),
    erlang:trace(Pid, true, [garbage_collection, {tracer, Self}]),
    Pid ! go,
    gc_events(Ref, 0).

traced(Collector, Loop, Args) ->
    receive go -> ok end,
    apply(?MODULE, Loop, [Collector | Args]).

gc_events(Ref, Count) ->
    receive
        {'DOWN', Ref, _, _, normal} -> Count;
        {trace, _, gc_minor_start, _} -> gc_events(Ref, Count + 1);
        {trace, _, gc_major_start, _} -> gc_events(Ref, Count + 1);
        _ -> gc_events(Ref, Count)
    end.
