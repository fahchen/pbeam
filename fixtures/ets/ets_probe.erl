-module(ets_probe).
-export([new/0, owner_exit/0, insert/0, lookup/0, lookup_loop/0, insert_loop/0,
         access/0, main/0, env/0, bag/0, info/0, match/0, match_deep/0,
         match_write_concurrency/0, delete_table/0, delete_trap/0,
         delete_order/0, deleter/2,
         insert_new/0, unsupported/1, insert_all/2, ping/1, busy/0, busy_insert/2,
         busy_helper/2,
         own/2, write/1, select/0, match_object/0, iterate/0,
         update_element/0, info_items/0, delete_all/0, select_reductions/0,
         private_owner/1, ordered_set/0, ordered_set_reductions/0, take/0]).

%% ets:new/2 return values and badarg cases (erl_db.c:2555-2840).
new() ->
    [kind(ets:new(ets_probe, [])),
     ets:new(ets_probe_named, [named_table]),
     catch_error(fun() -> ets:new(ets_probe_named, [named_table]) end),
     kind(ets:new(ets_probe_named, [])),
     ets:new(ets_probe, [set, protected, private, public, named_table,
                         {keypos, 2}, {read_concurrency, true},
                         {read_concurrency, false}, {write_concurrency, true},
                         {write_concurrency, false}, {write_concurrency, auto},
                         {write_concurrency, {debug_hash_fixed_number_of_locks, 1}},
                         {decentralized_counters, true},
                         {decentralized_counters, false}, compressed,
                         {heir, none}]),
     kind(ets:new(ets_probe, [{heir, dead()}, {heir, dead(), data}])),
     kind(ets:new(ets_probe, [{heir, self()}])),
     kind(ets:new(ets_probe, [{heir, self(), data}])),
     [catch_error(fun() -> ets:new(Name, Options) end)
      || {Name, Options} <-
             [{1, []}, {ets_probe, foo}, {ets_probe, [bogus]}, {ets_probe, [1]},
              {ets_probe, [{1, 2}]},
              {ets_probe, [{keypos, 0}]}, {ets_probe, [{keypos, a}]},
              {ets_probe, [{keypos, id(1 bsl 59)}]},
              {ets_probe, [{read_concurrency, yes}]},
              {ets_probe, [{write_concurrency, yes}]},
              {ets_probe, [{write_concurrency, {debug_hash_fixed_number_of_locks, 0}}]},
              {ets_probe, [{write_concurrency, {debug_hash_fixed_number_of_locks, id(32769)}}]},
              {ets_probe, [ordered_set, {write_concurrency, {debug_hash_fixed_number_of_locks, 1}}]},
              {ets_probe, [{decentralized_counters, yes}]},
              {ets_probe, [{heir, not_a_pid}]}, {ets_probe, [{heir, none, data}]},
              {ets_probe, [{keypos, 1, 2}]}, {ets_probe, [bag, bogus]},
              {ets_probe_named, [bag, named_table]}]]].

%% An exit of the owner deletes its tables and frees their names before
%% the DOWN message (erl_process.c:14360-14368, erl_db.c:5031-5093). A
%% heir that is the owner gets no table (erl_db.c:5076-5078).
owner_exit() ->
    Owner = start([named_table]),
    Heir = start(heir),
    [catch_error(fun() -> ets:new(ets_probe_owned, [named_table]) end),
     stop(Owner),
     ets:new(ets_probe_owned, [named_table]),
     catch_error(fun() -> ets:new(ets_probe_heir, [named_table]) end),
     stop(Heir),
     ets:new(ets_probe_heir, [named_table])].

%% ets:delete/1 (erl_db.c:3006-3082). The table loses its name and its
%% reference at once. Each process can delete a public table, and only the
%% owner can delete another table. Each other term raises badarg.
delete_table() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, {k, v}),
    ets_probe_named = ets:new(ets_probe_named, [named_table]),
    Public = start([public]),
    Protected = start([protected]),
    [ets:delete(T),
     catch_error(fun() -> ets:lookup(T, k) end),
     catch_error(fun() -> ets:delete(T) end),
     ets:delete(ets_probe_named),
     ets:info(ets_probe_named, type),
     ets:new(ets_probe_named, [named_table]),
     ets:delete(tid(Public)),
     catch_error(fun() -> ets:insert(tid(Public), {k, v}) end),
     stop(Public),
     catch_error(fun() -> ets:delete(tid(Protected)) end),
     ets:lookup(tid(Protected), k),
     stop(Protected),
     [catch_error(fun() -> ets:delete(Table) end)
      || Table <- [ets_probe_missing, make_ref(), 1, "t"]]].

%% A delete of a small table frees its segments in the turn of the caller.
%% A delete of a table with 50000 objects uses more reductions than a turn
%% has, so it traps (erl_db.c:3064-3077, 5378-5397), and a process that the
%% caller spawned runs before the delete returns. Run with +S 1.
delete_trap() ->
    Small = ets:new(ets_probe, []),
    true = ets:insert(Small, {k}),
    Large = ets:new(ets_probe, []),
    true = ets:insert(Large, [{I} || I <- seq(50000)]),
    receive after 1 -> ok end,
    spawn(?MODULE, ping, [self()]),
    A = ets:delete(Small),
    B = receive ping -> ping after 0 -> none end,
    C = ets:delete(Large),
    D = receive ping -> ping after 0 -> none end,
    [A, B, C, D].

%% The exit of a process that a trapped ets:delete/1 kills frees its owned
%% tables from the head of its owned list (erl_db.c:5055-5096). The deleted
%% table keeps its position in the list (erl_db.c:3031-3051), so an older
%% table goes first. A public table of another owner goes to the end of the
%% list (save_owned_table, erl_db.c:628-650), so a newer table goes first.
%% Each case gives the loop where a lookup of the small table first raises
%% badarg and the loop where the DOWN message arrives. Run with +S 1.
delete_order() ->
    Other = ets:new(ets_probe, [public]),
    true = ets:insert(Other, [{I} || I <- seq(50000)]),
    [kill_deleter(own), kill_deleter(Other)].

%% Make a small table, then delete the table Table, or a new large table
%% for own.
deleter(Parent, Table) ->
    Small = ets:new(ets_probe, [public]),
    true = ets:insert(Small, {k}),
    Large = case Table of
                own ->
                    T = ets:new(ets_probe, [public]),
                    true = ets:insert(T, [{I} || I <- seq(50000)]),
                    T;
                _ -> Table
            end,
    Parent ! {small, Small},
    ets:delete(Large).

kill_deleter(Table) ->
    Pid = spawn(?MODULE, deleter, [self(), Table]),
    Monitor = monitor(process, Pid),
    Small = receive {small, S} -> S end,
    exit(Pid, kill),
    order_loop(Small, Monitor, 0, undefined).

order_loop(_, _, 2000, Gone) -> {Gone, nodown};
order_loop(Small, Monitor, I, Gone) ->
    Now = case Gone of
              undefined ->
                  try ets:lookup(Small, k), undefined
                  catch error:badarg -> I
                  end;
              _ -> Gone
          end,
    receive {'DOWN', Monitor, _, _, _} -> {Now, I}
    after 0 -> erlang:yield(), order_loop(Small, Monitor, I + 1, Now)
    end.

%% Send ping to Pid.
ping(Pid) -> Pid ! ping.

%% Insert the list of objects Objects into the table T.
insert_all(T, Objects) -> ets:insert(T, Objects).

%% A list insert of 24000 objects into a table of the caller does its table
%% work in steps, and the table is busy between the steps
%% (`ETS_INSERT_2_LIST_GLOBAL`, erl_db.c:2077-2107, 2310-2319). A process
%% that uses the table then completes the insert with its own reductions
%% (`handle_lacking_permission`, erl_db.c:769-798), so its lookup finds the
%% last object, and the insert of the table returns `true` with the
%% reductions that it did not use (`erl_db.c:2223-2244`). The list is
%% `{I, I}` for 1 to 24000, which spans at least three turns. Run with +S 1.
busy() ->
    [busy_alone(), busy_helped(), busy_iteration()].

%% The insert of the list with no second process.
busy_alone() ->
    Parent = self(),
    Insert = spawn(?MODULE, busy_insert, [Parent, ets:new(ets_probe, [public])]),
    receive ready -> ok end,
    Insert ! go,
    receive M -> M end.

%% The insert of the list with a process that uses the table while it is
%% busy, at a higher priority, so it runs before the next turn of the
%% insert.
busy_helped() ->
    Parent = self(),
    Table = ets:new(ets_probe, [public]),
    Insert = spawn(?MODULE, busy_insert, [Parent, Table]),
    receive ready -> ok end,
    Insert ! go,
    %% Two turns of the insert: the check and the copy, then the lock of
    %% the table and the steps of the objects, which leave it busy.
    erlang:yield(),
    erlang:yield(),
    %% The table is busy here: the select completes the insert first and
    %% traps to its continuation (erl_db.c:769-798). An invalid
    %% specification traps too, and the resumed call raises badarg.
    Invalid = catch_error(fun() -> ets:select(Table, foo) end),
    Selected = ets:select(Table, [{{'$1', '_'}, [], ['$1']}]),
    _Helper = spawn_opt(?MODULE, busy_helper, [Parent, Table], [{priority, high}]),
    A = receive M -> M end,
    B = receive M2 -> M2 end,
    [Invalid, A, B, length(Selected)].

%% A process that uses the table while it is busy completes the insert in
%% its own turn, as `ets:next/2` does (`handle_lacking_permission`,
%% erl_db.c:769-798).
busy_iteration() ->
    Table = ets:new(ets_probe, [public]),
    Insert = spawn(?MODULE, busy_insert, [self(), Table]),
    receive ready -> ok end,
    Insert ! go,
    erlang:yield(),
    erlang:yield(),
    Taken = ets:take(Table, 2),
    receive {inserted, _, _} -> ok end,
    %% A second busy table: ets:next/2 completes its insert first
    %% (`handle_lacking_permission`, erl_db.c:769-798).
    Second = ets:new(ets_probe, [public]),
    Insert2 = spawn(?MODULE, busy_insert, [self(), Second]),
    receive ready -> ok end,
    Insert2 ! go,
    erlang:yield(),
    erlang:yield(),
    Next = ets:next(Second, 1),
    Info = length(ets:info(Table)),
    Updated = ets:update_element(Table, 1, {2, 9}),
    Deleted = ets:internal_select_delete(Table, [{{'_', '_'}, [false], [true]}]),
    Cleared = ets:internal_delete_all(Table, undefined),
    receive {inserted, _, _} -> ok end,
    [Taken, Next, Info, Updated, Deleted, Cleared].

%% Insert the list of busy_objects/0 into the table Table, and send the
%% result and the reductions of the insert to Parent.
busy_insert(Parent, Table) ->
    Objects = busy_objects(),
    Parent ! ready,
    receive go -> ok end,
    Before = reductions(),
    Result = ets:insert(Table, Objects),
    Parent ! {inserted, Result, reductions() - Before}.

%% Look up the last key of the list that the insert of another process
%% runs, and send the result and the reductions to Parent.
busy_helper(Parent, Table) ->
    Before = reductions(),
    Result = ets:lookup(Table, 24000),
    Parent ! {helped, Result, reductions() - Before}.

%% The objects of busy/0 and busy_alone/0: `{I, I}` for I in 1 to 24000.
busy_objects() ->
    [{I, I} || I <- seq(24000)].

%% The reductions that the calling process used so far.
reductions() ->
    element(2, process_info(self(), reductions)).

%% ets:insert/2 on a set table (erl_db.c:2341-2381, erl_db.c:1812-1827).
insert() ->
    T = ets:new(ets_probe, []),
    K = ets:new(ets_probe, [{keypos, 2}]),
    [ets:insert(T, {k, v}),
     ets:insert(T, [{k2, v2}, {k3, v3, w}, {k2, v4}]),
     ets:insert(T, []),
     ets:insert(T, [{k, v1}]),
     ets:insert(K, {v, k}),
     [catch_error(fun() -> ets:insert(Table, Objects) end)
      || {Table, Objects} <-
             [{T, x}, {T, {}}, {T, [x]}, {T, [{a, 1}, x]}, {K, {k}}, {K, [{k}]},
              {ets_probe_missing, []}, {ets_probe_missing, {a}}, {make_ref(), {a}}]],
     [ets:lookup_element(T, Key, 2, d) || Key <- [k, k2, k3, a]],
     ets:lookup_element(K, k, 1, d)].

%% ets:insert_new/2 in each table type (erl_db.c:2387-2440, erl_db.c:2017-2116,
%% erl_db_hash.c:1384-1446). An object whose key the table holds is not
%% inserted, and a list with such an object inserts no object. A key that
%% is two times in the list only is not in the table, so the list goes in.
insert_new() ->
    [insert_new(Type) || Type <- [set, bag, duplicate_bag]] ++ [insert_new_badarg()].

insert_new(Type) ->
    T = ets:new(ets_probe, [Type]),
    Results = [ets:insert_new(T, {k, 1}), ets:insert_new(T, {k, 2}),
               ets:insert_new(T, [{k, 3}]), ets:insert_new(T, []),
               ets:insert_new(T, [{a, 1}, {b, 1}, {a, 2}, {a, 2}, {a, 1}]),
               ets:insert_new(T, [{c, 1}, {d, 1}, {k, 4}]),
               ets:insert_new(T, [{e, 1}, {k, 5}, {f, 1}]),
               ets:insert_new(T, [{g, 1}, {h, 1}])],
    {Results, [ets:lookup(T, K) || K <- [k, a, b, c, d, e, f, g, h]]}.

insert_new_badarg() ->
    T = ets:new(ets_probe, []),
    K = ets:new(ets_probe, [{keypos, 2}]),
    true = ets:insert(T, {k, v}),
    Protected = start([protected]),
    Errors = [catch_error(fun() -> ets:insert_new(Table, Objects) end)
              || {Table, Objects} <-
                     [{T, x}, {T, {}}, {T, [x]}, {T, [{a, 1}, x]}, {T, [{a, 1} | x]},
                      {T, [{k, 1}, x]}, {T, [{a, 1}, {b, 1} | x]}, {K, {k}}, {K, [{k}]},
                      {K, [{a, k}, {k}]}, {ets_probe_missing, []},
                      {ets_probe_missing, {a}}, {ets_probe_missing, [{a}, {b}]},
                      {make_ref(), {a}}, {tid(Protected), []}, {tid(Protected), {a}},
                      {tid(Protected), [{a}, {b}]}, {tid(Protected), [{a}, x]}]],
    stop(Protected),
    {Errors, [ets:lookup(T, Key) || Key <- [a, b, k]], ets:lookup(K, a)}.

%% ets:lookup_element/4 (erl_db.c:2972-3001, erl_db_hash.c:1544-1604).
lookup() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{k, v}, {{1, k}, w, x}]),
    W = ets:new(ets_probe, [{keypos, id((1 bsl 32) + 2)}]),
    true = ets:insert(W, {a, k, v}),
    N = ets:new(ets_probe, [{keypos, id(1 bsl 31)}]),
    [ets:lookup_element(T, k, 2, d),
     ets:lookup_element(T, {1, k}, 3, d),
     ets:lookup_element(T, missing, 99, d),
     ets:lookup_element(T, k, (1 bsl 32) + 2, d),
     ets:lookup_element(T, missing, 1 bsl 32, d),
     ets:lookup_element(W, k, 3, d),
     catch_error(fun() -> ets:insert(N, {a, b}) end),
     ets:insert(N, []),
     ets:lookup_element(N, a, 1, d),
     [catch_error(fun() -> ets:lookup_element(Table, Key, Index, d) end)
      || {Table, Key, Index} <-
             [{T, missing, 0}, {T, k, 3}, {T, k, a}, {T, k, 1 bsl 31},
              {T, k, 1 bsl 59}, {ets_probe_missing, k, 1}, {make_ref(), k, 1},
              {1, k, 1}]]].

%% Access checks for a process that is not the owner (erl_db.c:769-798),
%% and the delete of the tables of an exiting owner (erl_db.c:5031-5093).
access() ->
    Tables = [{Options, start(Options)}
              || Options <- [[public], [protected], [private],
                             [private, protected], [private, public]]],
    [{_, Public} | _] = Tables,
    Named = start([named_table, public]),
    [[{catch_error(fun() -> ets:lookup_element(tid(Owner), k, 2, d) end),
       catch_error(fun() -> ets:insert(tid(Owner), {k, other}) end),
       catch_error(fun() -> ets:insert(tid(Owner), []) end)}
      || {_, Owner} <- Tables],
     catch_error(fun() -> ets:new(ets_probe_owned, [named_table]) end),
     ets:lookup_element(ets_probe_owned, k, 2, d),
     stop(Named),
     catch_error(fun() -> ets:lookup_element(ets_probe_owned, k, 2, d) end),
     ets:new(ets_probe_owned, [named_table]),
     ets:lookup_element(ets_probe_owned, k, 2, d),
     [stop(Owner) || {_, Owner} <- Tables],
     catch_error(fun() -> ets:lookup_element(tid(Public), k, 2, d) end)].

%% A loop of lookups in a set table of 1000 objects (#543). The table holds
%% the object {I, I * 2} for each I from 1 to 1000, and the loop reads each
%% key again, so the result is the sum of the values that the lookups read.
%% The compiled loop keeps the lookup work, as the disassembly of this
%% function shows.
lookup_loop() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{I, I * 2} || I <- seq(1000)]),
    lookup_loop(T, 0, 2000).

lookup_loop(_T, Sum, 0) ->
    Sum;
lookup_loop(T, Sum, N) ->
    [{_, Value}] = ets:lookup(T, (N rem 1000) + 1),
    lookup_loop(T, Sum + Value, N - 1).

%% The fill of lookup_loop/0 alone, so the program evidence shows the cost of
%% an insert as well as the cost of a lookup. The result is the object of the
%% last key, which shows that the table holds all 1000 objects.
insert_loop() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{I, I * 2} || I <- seq(1000)]),
    ets:lookup(T, 1000).

%% The integers 1 to `High` in descending order. PBeam has no
%% `lists:seq/2`, and a local loop keeps the table fill in the fixture.
seq(High) ->
    seq(1, High, []).

seq(Low, High, Acc) when Low > High ->
    Acc;
seq(Low, High, Acc) ->
    seq(Low + 1, High, [Low | Acc]).

%% PBeam has no application_controller, so main/0 makes ac_tab with the
%% options of application_controller.erl:544-545 in an owner process.
main() ->
    _ = start(ac_tab),
    env().

env() ->
    [application:get_env(kernel, standard_io_encoding, default),
     ets:insert(ac_tab, {{env, ets_probe, key}, value}),
     application:get_env(ets_probe, key, default)].

%% bag and duplicate_bag tables, and [bag, set], which makes a bag
%% (erl_db.c:2599-2611; erl_db_hash.c:1384-1664).
bag() ->
    [bag(Options) || Options <- [[bag], [duplicate_bag], [bag, set]]].

bag(Options) ->
    T = ets:new(ets_probe, Options),
    true = ets:insert(T, {k, 1}),
    true = ets:insert(T, {k, 2}),
    true = ets:insert(T, {k, 1}),
    true = ets:insert(T, [{k, 3}, {k, 2}, {k, 4, x}, {k, 3}]),
    true = ets:insert(T, [{j, 1}]),
    [ets:info(T, type),
     ets:lookup(T, k),
     ets:lookup_element(T, k, 2, d),
     ets:lookup_element(T, missing, 3, d),
     catch_error(fun() -> ets:lookup_element(T, k, 3, d) end),
     ets:member(T, k),
     ets:delete(T, k),
     ets:lookup(T, k),
     ets:member(T, k),
     ets:lookup(T, j)].

%% ets:info/2 for the item type (erl_db.c:4523-4579, 5447-5469).
info() ->
    T = ets:new(ets_probe, []),
    [ets:info(T, type),
     ets:info(ets:new(ets_probe, [public, bag]), type),
     ets:info(ets_probe_missing, type),
     ets:info(ets_probe_missing, bogus),
     [catch_error(fun() -> ets:info(Table, Item) end)
      || {Table, Item} <- [{T, bogus}, {T, 1}, {1, type}, {{T}, type}]]].

%% ets:match/2,3 on set, bag, and duplicate_bag tables (erl_db.c:3771-3818;
%% erl_db_hash.c:1835-1971, 2161-2307, 3177-3320).
match() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 3}, {1, x}, {2, y}, {"s", z},
                          {{1, 2}, v}, {<<"b">>, f}, {3, three}, {4, 4}]),
    M = ets:new(ets_probe, []),
    true = ets:insert(M, [{m, #{a => 1, b => 2}}, {n, #{a => 1}}, {o, [a]}]),
    [ets:match(T, '$1'),
     ets:match(T, {'$1', '$2'}),
     ets:match(T, {'$2', '$1'}),
     ets:match(T, {'$5', '_'}),
     ets:match(T, {'$0', '$1'}),
     ets:match(T, {'$1', '$1'}),
     ets:match(T, {a, '$1'}),
     ets:match(T, {a, '_'}),
     ets:match(T, {<<"b">>, '$1'}),
     ets:match(T, {missing, '$1'}),
     ets:match(T, {'_', 3}),
     ets:match(T, {'$1', ['$2' | '_']}),
     ets:match(T, foo),
     ets:match(T, {'_'}),
     ets:match(T, {'$_', '_'}),
     ets:match(T, {'$01', '_'}),
     ets:match(M, {m, #{a => '$1'}}),
     ets:match(M, {'$1', #{a => 1}}),
     ets:match(M, {'$1', #{b => '_'}}),
     ets:match(M, {'_', #{c => '_'}}),
     ets:match(T, {[$s], '$1'}),
     ets:match(T, '_'),
     ets:match(T, {{'$1', 2}, '_'}),
     ets:match(T, {[$t | '_'], '_'}),
     match(T, {'_', nomatch}, 1),
     ets:match(T, {['$1' | '_'], '_'}),
     [match(T, Pattern, Limit)
      || {Pattern, Limit} <- [{{'$1', '_'}, 3}, {{'$1', '_'}, 100},
                              {{missing, '_'}, 2}, {foo, 1}, {{a, '$1'}, 1}]],
     [catch_error(fun() -> ets:match(Table, Pattern) end)
      || {Table, Pattern} <- [{T, #{'$1' => a}}, {T, {'_', #{'_' => a}}},
                              {ets_probe_missing, '_'}, {1, '_'}]],
     [catch_error(fun() -> ets:match(Table, '_', Limit) end)
      || {Table, Limit} <- [{T, 0}, {T, -1}, {T, a}, {T, id(1 bsl 60)},
                            {ets_probe_missing, 1}]],
     catch_error(fun() -> ets:match(T, #{'$1' => a}, 1) end),
     match_float(),
     [match_bag(Type) || Type <- [bag, duplicate_bag]],
     match_large(),
     match_resumed(),
     match_private(),
     match_deleted()].

match(T, Pattern, Limit) ->
    case ets:match(T, Pattern, Limit) of
        {Matches, {Table, Slot, Chunk, Program, Rest, Size}} ->
            {Matches, Table =:= T, Slot, Chunk, is_reference(Program), Rest, Size};
        Result -> Result
    end.

%% Floats compare exactly: 1 and 1.0, and 0.0 and -0.0, are different
%% (erl_db_util.c:4160-4165).
match_float() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{1, 1.0}, {1.0, 1}, {2.0, 2.0}, {-0.0, z}, {0.0, y}]),
    [ets:match(T, {'$1', '$1'}), ets:match(T, {1.0, '$1'}), ets:match(T, {'$1', 1}),
     ets:match(T, {-0.0, '$1'}), ets:match(T, {'$1', 1.0})].

match_bag(Type) ->
    T = ets:new(ets_probe, [Type]),
    true = ets:insert(T, [{k, 1}, {k, 2}, {j, 1}, {k, 1, x}, {j, 2}, {k, 1}]),
    [ets:match(T, '$1'),
     ets:match(T, {k, '$1'}),
     ets:match(T, {'$1', 1}),
     ets:match(T, {k, '_', '$1'}),
     match(T, {k, '$1'}, 1),
     match(T, '$1', 2)].

%% A match of more than 1000 objects traps, and the result keeps the order
%% of the buckets (erl_db_hash.c:1938-1950, 2264-2307).
match_large() ->
    T = ets:new(ets_probe, [public]),
    true = ets:insert(T, [{I, I * 2} || I <- seq(5000)]),
    true = ets:delete(T, 7),
    All = ets:match(T, {'$1', '$2'}),
    {Chunk, _} = ets:match(T, {'$1', '_'}, 2000),
    [length(All), take(5, All), checksum(All, 0), length(Chunk), checksum(Chunk, 0)].

%% After a trap, the rest of ets:match/3 is in reverse order
%% (erl_db_hash.c:2380-2388).
match_resumed() ->
    T = ets:new(ets_probe, [duplicate_bag]),
    true = ets:insert(T, [{K, V} || K <- seq(500), V <- seq(10)]),
    {Matches, {_, Slot, Chunk, _, Rest, Size}} = ets:match(T, {'$1', '$2'}, 1001),
    [length(Matches), checksum(Matches, 0), Slot, Chunk, Rest, Size].

%% A pattern 1000000 tuples deep matches an object of the same depth, as
%% the work stack of db_match_compile and the match program let it
%% (erl_db_util.c:1700-2100).
match_deep() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, {k, nest(1000000, x)}),
    ets:match(T, {k, nest(1000000, '$1')}).

nest(0, Term) -> Term;
nest(N, Term) -> nest(N - 1, {Term}).

%% With one scheduler, a table has no fine locks, so write_concurrency
%% does not change the traversal order (erl_db.c:2656-2657;
%% erl_db_util.h:388-389; erl_db_hash.c:1023-1030, 539-554).
match_write_concurrency() ->
    T = ets:new(ets_probe, [set, {write_concurrency, true}]),
    true = ets:insert(T, [{I} || I <- seq(300)]),
    All = ets:match(T, {'$1'}),
    [length(All), take(6, All), checksum(All, 0)].

take(0, _) -> [];
take(N, [X | Xs]) -> [X | take(N - 1, Xs)].

%% An order-sensitive checksum of the keys of a match result.
checksum([], Sum) -> Sum;
checksum([[Key | _] | Rest], Sum) -> checksum(Rest, (Sum * 31 + Key) rem 1000000007).

%% The owner exit deletes the table while the match traps, and the next
%% turn raises badarg (erl_db.c:3883-3897).
match_deleted() ->
    Owner = start([public]),
    true = ets:insert(tid(Owner), [{I} || I <- seq(50000)]),
    element(1, Owner) ! stop,
    catch_error(fun() -> ets:match(tid(Owner), '$1') end).

match_private() ->
    Owner = start([private]),
    Result = catch_error(fun() -> ets:match(tid(Owner), '_') end),
    stopped = stop(Owner),
    Result.

%% Calls that ERTS runs and PBeam does not support yet.
unsupported(Case) ->
    case Case of
        match_1 ->
            T = ets:new(ets_probe, []),
            true = ets:insert(T, [{a}, {b}]),
            ets:match(element(2, ets:match(T, '$1', 1)));
        match_changed ->
            T = ets:new(ets_probe, [public]),
            true = ets:insert(T, [{I} || I <- seq(5000)]),
            spawn(?MODULE, write, [T]) ! go,
            ets:match(T, '$1');
        delete_inserting ->
            T = ets:new(ets_probe, [public]),
            spawn(?MODULE, insert_all, [T, [{I} || I <- seq(400000)]]),
            true = ets:insert(ets:new(ets_probe, []), [{I} || I <- seq(40000)]),
            ets:delete(T);
        info_size -> ets:info(ets:new(ets_probe, []), size);
        info_binary -> ets:info(ets:new(ets_probe, []), binary);
        info_reference -> ets:info(make_ref(), type);
        info_all_reference -> ets:info(make_ref());
        info_stats -> ets:info(ets:new(ets_probe, []), stats);
        info_data -> ets:info(ets:new(ets_probe, []), data);
        select_unsupported ->
            T = ets:new(ets_probe, []),
            true = ets:insert(T, {a, 1}),
            ets:select(T, [{{'$1', '$2'}, [], [{round, 1.0}]}]);
        select_delete_unsupported ->
            T = ets:new(ets_probe, []),
            true = ets:insert(T, {a, 1}),
            ets:internal_select_delete(T, [{{'$1', '$2'}, [], [{round, 1.0}]}]);
        ordered_next ->
            T = ets:new(ets_probe, [ordered_set]),
            true = ets:insert(T, {a, 1}),
            ets:next(T, a);
        ordered_select ->
            T = ets:new(ets_probe, [ordered_set]),
            true = ets:insert(T, {a, 1}),
            ets:select(T, [{{'$1', '$2'}, [], ['$1']}]);
        heir -> ets:new(ets_probe, [{heir, element(1, start([]))}]);
        heir_data -> ets:new(ets_probe, [{heir, element(1, start([])), data}]);
        keypos -> ets:insert(ets:new(ets_probe, [{keypos, id(1 bsl 32)}]), {a});
        keypos_new -> ets:insert_new(ets:new(ets_probe, [{keypos, id(1 bsl 32)}]), {a});
        index ->
            T = ets:new(ets_probe, []),
            true = ets:insert(T, {k, v}),
            ets:lookup_element(T, k, 1 bsl 32, d)
    end.

%% Insert one object into the table T when the message go comes.
write(T) ->
    receive
        go -> ets:insert(T, {x})
    end.

start(Options) ->
    Parent = self(),
    Owner = spawn(?MODULE, own, [Parent, Options]),
    receive
        {Owner, Table} -> {Owner, Table}
    end.

own(Parent, ac_tab) ->
    own(Parent, ac_tab, [set, public, named_table, {read_concurrency, true}]);
own(Parent, heir) ->
    own(Parent, ets_probe_heir, [named_table, {heir, self(), data}]);
own(Parent, Options) ->
    Name = case lists:member(named_table, Options) of
               true -> ets_probe_owned;
               false -> ets_probe
           end,
    own(Parent, Name, Options).

own(Parent, Name, Options) ->
    Table = ets:new(Name, Options),
    true = ets:insert(Table, {k, owner}),
    Parent ! {self(), Table},
    receive
        stop -> ok
    end.

tid({_, Table}) -> Table.

kind(Table) when is_reference(Table) -> reference;
kind(Table) -> Table.

stop({Owner, _}) ->
    Ref = monitor(process, Owner),
    Owner ! stop,
    receive
        {'DOWN', Ref, process, Owner, normal} -> stopped
    end.

dead() ->
    {Pid, _} = start([]),
    stopped = stop({Pid, x}),
    Pid.

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.

%% Keep an integer outside the 32-bit signed range out of a literal: PBeam
%% does not load SMALL_BIG_EXT integers in literals.
id(X) -> X.


%% ets:select/1,2,3 on set, bag, and duplicate_bag tables
%% (erl_db.c:3821-4048; erl_db_hash.c:1835-2478).
select() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 3}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(B, [{a, 1}, {a, 2}, {b, 3}]),
    D = ets:new(ets_probe, [duplicate_bag]),
    true = ets:insert(D, [{a, 1}, {a, 1}, {b, 2}]),
    [ets:select(T, [{{'$1', '$2'}, [], ['$1']}]),
     ets:select(T, [{{'$1', '$2'}, [], ['$_']}]),
     ets:select(T, [{{'$1', '$2'}, [], ['$$']}]),
     ets:select(T, [{{'$1', '$2'}, [{'>', '$2', 1}], ['$1']}]),
     ets:select(T, [{{'$1', '$2'}, [{element, 2, '$_'}], ['$1']}]),
     ets:select(T, [{{'$1', '$2'}, [{'=:=', {element, 2, '$_'}, {const, 2}}], ['$1']}]),
     ets:select(T, [{{'$1', '$2'}, [], [{{'$1', '$2'}}]}]),
     catch_error(fun() -> ets:select(T, [{{'$1', '$2'}, [], [{'$2', '$1'}]}]) end),
     ets:select(T, [{{'$1', '$2'}, [{is_integer, '$2'}], ['$2']},
                    {{'$1', '$2'}, [{'=:=', '$1', {const, c}}], ['$2']}]),
     ets:select(B, [{{'$1', '$2'}, [], ['$1']}]),
     ets:select(D, [{'$1', [], ['$_']}]),
     ets:select(T, [{{'$1', '$2'}, [], [{'+', '$2', 1}]}]),
     ets:select(T, [{{'$1', '$2'}, [], [{element, 3, '$_'}]}]),
     catch_error(fun() -> ets:select(T, foo) end),
     catch_error(fun() -> ets:select(T, [{{'$1', '$2'}, [{bad_guard, 1}], ['$1']}]) end),
     catch_error(fun() -> ets:select(T, [{{'$1', '$2'}, [], [{bad_body, 1}]}]) end),
     catch_error(fun() -> ets:select(T, [{{'$1', '$2'}, [], [{'$1', '$2'}]}]) end),
     catch_error(fun() -> ets:select(T, [{{'$1', '$2'}, [], ['$3']}]) end),
     select_chunks(T),
     select_continue_errors(T),
     [catch_error(fun() -> ets:select(Table, Spec) end)
      || {Table, Spec} <- [{ets_probe_missing, [{'$1', [], ['$1']}]},
                           {1, [{'$1', [], ['$1']}]},
                           {T, #{'$1' => a}}]],
     [catch_error(fun() -> ets:select(Table, [{'$1', [], ['$1']}], Limit) end)
      || {Table, Limit} <- [{T, 0}, {T, -1}, {T, a}, {ets_probe_missing, 1}]],
     begin
         {PrivateOwner, Private, PrivateContinuation} = private_table(),
         Result = [catch_error(fun() -> ets:select(Private, [{'$1', [], ['$1']}]) end),
                   catch_error(fun() -> ets:select(PrivateContinuation) end)],
         PrivateOwner ! stop,
         Result
     end].

%% A limited select and its continuation. The continuation is immutable:
%% the same continuation gives the same rest again
%% (erl_db_hash.c:2224-2254, 2379-2401).
select_chunks(T) ->
    {First, C} = ets:select(T, [{'$1', [], ['$1']}], 2),
    {All, End} = ets:select(T, [{'$1', [], ['$1']}], 10),
    {Table, _Next, Limit, Program, _Rest, _Size} = C,
    [First, ets:select(C), ets:select(C), All, End, ets:select(End),
     ets:select({Table, -1, Limit, Program, [first], 2})].

%% Each continuation that is not a live 6-tuple raises badarg
%% (erl_db_hash.c:2411-2478).
select_continue_errors(T) ->
    {_, C} = ets:select(T, [{'$1', [], ['$1']}], 2),
    {Table, _Next, Limit, Program, Rest, Size} = C,
    [catch_error(fun() -> ets:select(foo) end),
     catch_error(fun() -> ets:select({}) end),
     catch_error(fun() -> ets:select({T, 0, 1, make_ref(), [], x}) end),
     catch_error(fun() -> ets:select({T, 0, -1, make_ref(), [], 0}) end),
     catch_error(fun() -> ets:select({T, 0, 1, make_ref(), bad, 0}) end),
     catch_error(fun() -> ets:select({T, 0, 1, make_ref(), [], -1}) end),
     catch_error(fun() -> ets:select({T, 0, 1, make_ref(), [], 0}) end),
     catch_error(fun() -> ets:select({ets_probe_missing, 0, 1, make_ref(), [], 0}) end),
     catch_error(fun() -> ets:select({Table, 999, Limit, Program, Rest, Size}) end)].

%% ets:match_object/2 (erl_db.c:4343-4362).
match_object() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 3}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(B, [{a, 1}, {a, 2}, {b, 3}]),
    [ets:match_object(T, '_'),
     ets:match_object(T, {'$1', '$2'}),
     ets:match_object(T, {a, '_'}),
     ets:match_object(T, {'_', 2}),
     ets:match_object(B, {'$1', '$2'}),
     catch_error(fun() -> ets:match_object(ets_probe_missing, '_') end)].

%% ets:next/2 and ets:prev/2, which give the hash chain order; prev is next
%% for a hash table (erl_db_hash.c:854-856, 1147-1200).
iterate() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 3}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(B, [{a, 1}, {a, 2}, {b, 3}]),
    Keys = fun ets_next/2,
    {PrivateOwner, Private, _} = private_table(),
    Result = [Keys(T, a), Keys(T, b), Keys(T, c), Keys(B, a), Keys(B, b),
              catch_error(fun() -> ets:next(T, missing) end),
              catch_error(fun() -> ets:prev(T, missing) end),
              ets:prev(T, a), ets:prev(T, b), ets:prev(T, c),
              catch_error(fun() -> ets:next(Private, a) end),
              catch_error(fun() -> ets:prev(Private, a) end)],
    PrivateOwner ! stop,
    Result.

ets_next(Table, Key) ->
    [ets:next(Table, Key), ets:prev(Table, Key)].

%% Return the owner and the reference of a private table of another
%% process, and a continuation of an `ets:select/3` of that process. Send
%% `stop` to the owner when the caller is done.
private_table() ->
    Owner = spawn(?MODULE, private_owner, [self()]),
    receive {private_table, T, C} -> {Owner, T, C} end.

private_owner(Parent) ->
    T = ets:new(ets_probe_private, [private]),
    true = ets:insert(T, [{a, 1}]),
    {_, C} = ets:select(T, [{'$1', [], ['$1']}], 1),
    Parent ! {private_table, T, C},
    receive stop -> ok end.

%% ets:update_element/3,4 (erl_db.c:1371-1491).
update_element() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1, x}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(B, [{a, 1}]),
    [ets:update_element(T, a, {2, 9}),
     ets:lookup(T, a),
     ets:update_element(T, a, [{2, 8}, {2, 7}, {3, z}]),
     ets:lookup(T, a),
     ets:update_element(T, missing, {2, 9}),
     ets:update_element(T, missing, {2, 9}, {missing, 0, d}),
     ets:lookup(T, missing),
     ets:update_element(T, a, [], {ignored}),
     catch_error(fun() -> ets:update_element(B, a, {2, 9}) end),
     catch_error(fun() -> ets:update_element(T, a, {1, 9}) end),
     catch_error(fun() -> ets:update_element(T, a, {4, 9}) end),
     catch_error(fun() -> ets:update_element(T, a, {0, 9}) end),
     catch_error(fun() -> ets:update_element(T, a, bad) end),
     catch_error(fun() -> ets:update_element(T, a, [{2, 9}, bad]) end),
     catch_error(fun() -> ets:update_element(T, a, [{2, 9}, {2}]) end),
     catch_error(fun() -> ets:update_element(T, a, {2, 9}, bad) end),
     catch_error(fun() -> ets:update_element(T, a, {2, 9}, {a}) end),
     catch_error(fun() -> ets:update_element(T, new, {3, x}, {new, 0}) end),
     ets:lookup(T, new),
     catch_error(fun() -> ets:update_element(T, missing, bad) end),
     begin
         K = ets:new(ets_probe, [{keypos, 1 bsl 31}]),
         catch_error(fun() -> ets:update_element(K, k, {2, x}, {k, 0}) end)
     end,
     catch_error(fun() -> ets:update_element(T, a, [], bad) end),
     catch_error(fun() -> ets:update_element(ets_probe_missing, a, {2, 9}) end),
     begin
         {PrivateOwner, Private, _} = private_table(),
         Result = [catch_error(fun() -> ets:update_element(Private, a, {2, 9}) end),
                   catch_error(fun() -> ets:update_element(Private, a, {2, 9}, {a, 0}) end)],
         PrivateOwner ! stop,
         Result
     end].

%% ets:info/1,2 (erl_db.c:4399-4579, 5447-5625). The owner, the id, and
%% the memory depend on the run, so the list omits them.
info_items() ->
    T = ets:new(ets_probe, [named_table, public, {keypos, 1},
                            {read_concurrency, true}, {write_concurrency, true},
                            {decentralized_counters, true}, compressed]),
    true = ets:insert(T, [{a, 1}]),
    U = ets:new(ets_probe, []),
    [[element(1, Pair) || Pair <- ets:info(T)],
     [{K, V} || {K, V} <- ets:info(T), not lists:member(K, [owner, id, memory])],
     [ets:info(T, I) || I <- [type, size, heir, protection, write_concurrency,
                              read_concurrency, name, keypos, node, named_table,
                              compressed, decentralized_counters, fixed,
                              safe_fixed, safe_fixed_monotonic_time]],
     [ets:info(U, I) || I <- [name, named_table, type, size]],
     catch_error(fun() -> ets:info(1) end),
     ets:info(ets_probe_missing),
     begin
         {PrivateOwner, Private, _} = private_table(),
         Result = ets:info(Private, protection),
         PrivateOwner ! stop,
         Result
     end,
     ets:info(ets_probe_missing, type),
     ets:info(ets_probe_missing, size),
     [catch_error(fun() -> ets:info(Table, Item) end)
      || {Table, Item} <- [{T, bogus}, {T, 1}, {1, type}, {{T}, type}]]].

%% ets:delete_all_objects/1, ets:match_delete/2, and ets:select_delete/2
%% through ets:internal_delete_all/2 and ets:internal_select_delete/2
%% (lib/stdlib/src/ets.erl:386-396, 1849-1861, 2761-2763;
%% erl_db.c:3234-3299, 3433-3475).
delete_all() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 2}]),
    R1 = [ets:delete_all_objects(T), ets:info(T, size)],
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 2}]),
    R2 = [ets:internal_delete_all(T, undefined), ets:info(T, size)],
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 2}]),
    R3 = [ets:match_delete(T, {'_', 2}), ets:lookup(T, a)],
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 2}]),
    R4 = [ets:select_delete(T, [{{'$1', '$2'}, [{'>', '$2', 1}], [true]}]),
          ets:lookup(T, a)],
    D = ets:new(ets_probe, [duplicate_bag]),
    true = ets:insert(D, [{a, 1}, {a, 2}, {a, 3}]),
    R5 = [ets:select_delete(D, [{{a, '$1'}, [{'=:=', '$1', {const, 3}}], [true]}]),
          ets:select_delete(D, [{{a, '$1'}, [{'=:=', '$1', {const, 2}}], [true]}]),
          ets:lookup(D, a)],
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 2}]),
    R6 = ets:internal_delete_all(T, holder),
    R7 = ets:internal_select_delete(T, [{foo, [], [true]}]),
    R8 = [ets:delete_all_objects(T), ets:info(T, size)],
    {PrivateOwner, Private, _} = private_table(),
    R9 = [ets:info(Private, owner) =:= self(),
          catch_error(fun() -> ets:insert(Private, {b, 2}) end),
          catch_error(fun() -> ets:internal_select_delete(Private, [{{'_', '_'}, [], [true]}]) end),
          catch_error(fun() -> ets:internal_delete_all(Private, undefined) end)],
    PrivateOwner ! stop,
    [R1, R2, R3, R4, R5, R6, R7, R8, R9,
     catch_error(fun() -> ets:internal_select_delete(T, foo) end),
     catch_error(fun() -> ets:internal_select_delete(ets_probe_missing, []) end),
     catch_error(fun() -> ets:internal_delete_all(ets_probe_missing, undefined) end)].

%% The reductions of the new BIFs. Run with +S 1.
select_reductions() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 3}]),
    Call = fun(F) -> Before = reductions(), F(), reductions() - Before end,
    [Call(fun() -> ets:select(T, [{'$1', [], ['$1']}]) end),
     Call(fun() -> ets:select(T, [{'$1', [], ['$1']}], 1) end),
     Call(fun() -> ets:next(T, a) end),
     Call(fun() -> ets:update_element(T, a, {2, 9}) end),
     Call(fun() -> ets:internal_select_delete(T, [{'$1', [], [false]}]) end),
     Call(fun() -> ets:internal_delete_all(T, undefined) end)].

%% An `ordered_set` table keeps the hash structure of PBeam, so its
%% single-key operations give the ERTS result, and a key-order operation
%% raises a clear runtime error (see "ETS tables" in docs/erts-differences.md).
ordered_set() ->
    T = ets:new(ets_probe, [ordered_set, public]),
    Insert = ets:insert(T, [{b, 2}, {a, 1}, {c, 3}, {a, 10}]),
    Lookup = ets:lookup(T, a),
    Member = [ets:member(T, a), ets:member(T, z)],
    New = [ets:insert_new(T, [{d, 4}]), ets:insert_new(T, [{a, 1}])],
    Info = [ets:info(T, type), ets:info(T, size), ets:info(T, keypos),
            ets:info(T, protection)],
    Taken = ets:take(T, b),
    TakenAgain = ets:take(T, b),
    Deleted = [ets:delete(T, a), ets:info(T, size)],
    Left = ets:info(T, size),
    %% An `ordered_set` compares keys with the arithmetic term order
    %% (`cmp_key_eq`, `erl_db_tree.c:758-761`), so `1` and `1.0` are one
    %% key, also in a tuple or a list, and map values compare by their mode.
    true = ets:insert(T, [{1, one}, {{2, a}, two}, {[1, b], list}]),
    Arithmetic = [ets:lookup(T, 1.0), ets:member(T, 1.0),
                  ets:insert_new(T, {1.0, other}),
                  ets:lookup_element(T, 1.0, 2, undefined),
                  ets:update_element(T, 1.0, {2, updated}),
                  ets:lookup(T, 1),
                  ets:take(T, 1.0),
                  ets:lookup(T, 1),
                  ets:lookup(T, {2.0, a}),
                  ets:lookup(T, [1.0, b]),
                  ets:delete(T, {2.0, a}),
                  ets:lookup(T, {2, a})],
    [Insert, Lookup, Member, New, Info, Taken, TakenAgain, Deleted, Left,
     Arithmetic].

%% The reductions of the single-key operations and of
%% `ets:internal_delete_all/2` of an `ordered_set` table. The ERTS tree
%% frees one node for each reduction (`do_free_tree_continue`,
%% `erl_db_tree.c:2877-2904`), and the single-key operations charge the
%% reductions of the tree.
ordered_set_reductions() ->
    T = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(T, [{a, 1}, {b, 2}, {c, 3}]),
    Call = fun(F) -> Before = reductions(), F(), reductions() - Before end,
    [Call(fun() -> ets:lookup(T, a) end),
     Call(fun() -> ets:lookup(T, a) end),
     Call(fun() -> ets:insert(T, {d, 4}) end),
     Call(fun() -> ets:update_element(T, a, {2, 9}) end),
     Call(fun() -> ets:take(T, b) end),
     Call(fun() -> ets:delete(T, a) end),
     Call(fun() -> ets:internal_delete_all(T, undefined) end)].

%% `ets:take/2` on the three hash table types.
take() ->
    T = ets:new(ets_probe, []),
    true = ets:insert(T, [{a, 1}, {b, 2}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(B, [{a, 1}, {a, 2}, {b, 3}]),
    D = ets:new(ets_probe, [duplicate_bag]),
    true = ets:insert(D, [{a, 1}, {a, 1}, {b, 2}]),
    Invalid = catch_error(fun() -> ets:take(make_ref(), a) end),
    [Invalid, ets:take(T, a), ets:lookup(T, a), ets:take(T, missing),
     ets:take(B, a), ets:lookup(B, a), ets:take(B, a),
     ets:take(D, a), ets:take(D, a), ets:lookup(D, a)].
