-module(ets_probe).
-export([heir_transfer/0, give_away/0, setopts/0, delete_object/0, new/0, owner_exit/0, insert/0, lookup/0, lookup_loop/0, insert_loop/0,
         access/0, main/0, env/0, bag/0, info/0, match/0, match_deep/0,
         match_write_concurrency/0, delete_table/0, delete_trap/0,
         delete_order/0, deleter/2,
         insert_new/0, unsupported/1, insert_all/2, ping/1, busy/0, busy_insert/2,
         busy_helper/2,
         own/2, write/1, select/0, match_object/0, iterate/0,
         update_element/0, info_items/0, delete_all/0, select_reductions/0, first/0,
         private_owner/1, ordered_set/0, ordered_set_reductions/0, take/0,
         walk/0, walk_alias/1,
         safe_fixtable/0, fixtable/0, fixtable_chain/0, fixtable_successor/0,
         fixtable_resize/0, fixtable_fix_count/0, fixtable_insert_after_delete/0,
         fixtable_clear/0, fixtable_delete_object/0, fixtable_same_bucket/0,
         fixtable_clear_wide/0, fixtable_live_after_predecessor/0, fixtable_cycle/0,
         fixtable_insert_before_successor/0, fixtable_reinsert/0,
         fixtable_clear_reinsert/0, fixtable_clear_after_delete/0,
         fixtable_bag_reinsert/0, fixtable_shrink/0,
         fixtable_duplicate_bag_reinsert/0, fixtable_info/0,
         fixtable_ordered_set/0, fixtable_safe_fixed/0,
         fixtable_bag_repeat/0, fixtable_bag_partial/0,
         fixtable_bag_double_delete/0, fixtable_bag_restore/0,
         fixtable_bag_last_live/0, fixtable_bag_skip_deleted/0,
         fixtable_next_live_behind_node/0, fixtable_clear_behind_node/0,
         fixtable_bag_restore_behind_node/0, fixtable_delete_table/0,
         fixtable_clear_large/0, fixtable_clear_help/0, clear_task/2,
         fixtable_take/0, fixtable_take_clear/0, fixtable_insert_new/0,
         fixtable_insert_new_list/0, ordered_select/0, select_delete_worker/2, ordered_select_large/0,
         select_reverse/0,
         update_counter/0]).

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
    Ordered = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(Ordered, [{I} || I <- seq(50000)]),
    receive after 1 -> ok end,
    spawn(?MODULE, ping, [self()]),
    A = ets:delete(Small),
    B = receive ping -> ping after 0 -> none end,
    C = ets:delete(Large),
    D = receive ping -> ping after 0 -> none end,
    spawn(?MODULE, ping, [self()]),
    E = ets:delete(Ordered),
    F = receive ping -> ping after 0 -> none end,
    [A, B, C, D, E, F].

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
    %% A third busy table: ets:safe_fixtable/2 completes its insert first
    %% and then gives true.
    Third = ets:new(ets_probe, [public]),
    Insert3 = spawn(?MODULE, busy_insert, [self(), Third]),
    receive ready -> ok end,
    Insert3 ! go,
    erlang:yield(),
    erlang:yield(),
    Fixed = ets:safe_fixtable(Third, true),
    receive {inserted, _, _} -> ok end,
    Info = length(ets:info(Table)),
    Updated = ets:update_element(Table, 1, {2, 9}),
    Deleted = ets:internal_select_delete(Table, [{{'_', '_'}, [false], [true]}]),
    Cleared = ets:internal_delete_all(Table, undefined),
    receive {inserted, _, _} -> ok end,
    [Taken, Next, Fixed, Info, Updated, Deleted, Cleared].

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
              {1, k, 1}]],
     [ets:lookup_element(T, k, 2),
      ets:lookup_element(T, {1, k}, 3),
      ets:lookup_element(W, k, 3),
      catch_info(fun() -> ets:lookup_element(T, missing, 1) end),
      catch_info(fun() -> ets:lookup_element(T, k, 3) end),
      catch_error(fun() -> ets:lookup_element(T, missing, 1) end),
      catch_error(fun() -> ets:lookup_element(T, k, 0) end),
      catch_error(fun() -> ets:lookup_element(T, k, a) end),
      catch_error(fun() -> ets:lookup_element(ets_probe_missing, k, 1) end),
      catch_error(fun() -> ets:lookup_element(make_ref(), k, 1) end)]].

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
            erlang:yield(),
            ets:delete(T);
        info_size -> ets:info(ets:new(ets_probe, []), size);
        info_binary -> ets:info(ets:new(ets_probe, []), binary);
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


%% ets:safe_fixtable/2 results and badarg cases (erl_db.c:1136-1164).
safe_fixtable() ->
    T = ets:new(ets_probe_fixtable, [set]),
    [ets:safe_fixtable(T, true), ets:safe_fixtable(T, true),
     ets:safe_fixtable(T, false), ets:safe_fixtable(T, false),
     catch_error(fun() -> ets:safe_fixtable(T, 1) end),
     catch_error(fun() -> ets:safe_fixtable(T, yes) end),
     catch_error(fun() -> ets:safe_fixtable(no_such_table, true) end),
     catch_error(fun() -> ets:safe_fixtable(1, true) end)].


%% A fixed table keeps the key of a deleted object for ets:next/2
%% (erl_db_hash.c:49-60); an unfixed table gives badarg for the key.
fixtable() ->
    T = ets:new(ets_probe_fix, [set]),
    ets:insert(T, {k}),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    Fixed = ets:next(T, k),
    ets:safe_fixtable(T, false),
    Unfixed = catch_error(fun() -> ets:next(T, k) end),
    [Fixed, Unfixed].


%% A fixed table follows the ghosts of two deleted keys, and an insert or a
%% delete while fixed does not resize the table (erl_db_hash.c:49-60).
fixtable_chain() ->
    T = ets:new(ets_probe_fix_chain, [set]),
    ets:insert(T, [{1}, {2}, {3}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 1),
    ets:delete(T, 2),
    ets:delete(T, 3),
    Fixed = ets:next(T, 1),
    ets:safe_fixtable(T, false),
    [Fixed].

fixtable_successor() ->
    T = ets:new(ets_probe_fix_successor, [set]),
    ets:insert(T, [{1}, {2}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 1),
    Next = ets:next(T, 1),
    ets:safe_fixtable(T, false),
    [Next].

fixtable_resize() ->
    T = ets:new(ets_probe_fix_resize, [set]),
    ets:insert(T, [{I} || I <- from(1, 5000)]),
    ets:safe_fixtable(T, true),
    ets:insert(T, [{I} || I <- from(5001, 10000)]),
    Before = ets:info(T, size),
    [ets:delete(T, I) || I <- from(1, 10000)],
    After = ets:info(T, size),
    ets:safe_fixtable(T, false),
    [Before, After].

from(High, High) -> [High];
from(Low, High) -> [Low | from(Low + 1, High)].


%% Each fix counts, so one unfix leaves the table fixed
%% (erl_db.c:5158-5169).
fixtable_fix_count() ->
    T = ets:new(ets_probe_fix_count, [set]),
    ets:insert(T, {k}),
    ets:safe_fixtable(T, true),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    ets:safe_fixtable(T, false),
    Fixed = ets:next(T, k),
    ets:safe_fixtable(T, false),
    [Fixed].

%% A key that the run inserts after the delete is still after the fixed
%% position of the deleted key.
fixtable_insert_after_delete() ->
    T = ets:new(ets_probe_fix_insert, [set]),
    ets:insert(T, {1}),
    ets:safe_fixtable(T, true),
    ets:delete(T, 1),
    ets:insert(T, {2}),
    Next = ets:next(T, 1),
    ets:safe_fixtable(T, false),
    [Next].

%% A clear and a delete_object keep the position of each removed key
%% (erl_db_hash.c:2636-2643, 4027-4028).
fixtable_clear() ->
    T = ets:new(ets_probe_fix_clear, [set]),
    ets:insert(T, {k}),
    ets:safe_fixtable(T, true),
    ets:internal_delete_all(T, undefined),
    Next = ets:next(T, k),
    ets:safe_fixtable(T, false),
    [Next].

fixtable_delete_object() ->
    T = ets:new(ets_probe_fix_delete_object, [set]),
    ets:insert(T, {k}),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{'_'}, [], [true]}]),
    Next = ets:next(T, k),
    ets:safe_fixtable(T, false),
    [Next].


%% Two keys of one bucket: the ghost follows the live predecessor, and then
%% the ghost of a predecessor that the run deleted too.
fixtable_same_bucket() ->
    T = ets:new(ets_probe_fix_bucket, [set]),
    ets:insert(T, [{2}, {146}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 146),
    Next = ets:next(T, 146),
    ets:delete(T, 2),
    Chained = ets:next(T, 146),
    ets:safe_fixtable(T, false),
    [Next, Chained].

%% Two ghosts that follow each other end at the end of the table.
fixtable_cycle() ->
    T = ets:new(ets_probe_fix_cycle, [set]),
    ets:insert(T, [{2}, {146}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 146),
    ets:insert(T, {146}),
    ets:delete(T, 2),
    ets:delete(T, 146),
    Next = ets:next(T, 2),
    ets:safe_fixtable(T, false),
    [Next].

%% The deleted key follows its live predecessor, and a key that the run
%% inserts after the predecessor is after it.
fixtable_live_after_predecessor() ->
    T = ets:new(ets_probe_fix_live, [set]),
    ets:insert(T, [{4}, {84}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 84),
    ets:insert(T, {136}),
    Next = ets:next(T, 84),
    ets:safe_fixtable(T, false),
    [Next].

%% A clear of a table with more buckets than a new table keeps the
%% position of each removed key (erl_db_hash.c:4027-4028).
fixtable_clear_wide() ->
    T = ets:new(ets_probe_fix_clear_wide, [set]),
    ets:insert(T, [{I} || I <- from(1, 1000)]),
    ets:safe_fixtable(T, true),
    ets:internal_delete_all(T, undefined),
    Ended = length([1 || I <- from(1, 1000), ets:next(T, I) =:= '$end_of_table']),
    ets:safe_fixtable(T, false),
    [Ended].


%% A key that the run inserts into a bucket between the deleted key and
%% its successor is after the deleted key.
fixtable_insert_before_successor() ->
    T = ets:new(ets_probe_fix_before, [set]),
    ets:insert(T, [{1}, {2}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 1),
    ets:insert(T, {17}),
    Next = ets:next(T, 1),
    ets:safe_fixtable(T, false),
    [Next].

%% The insert of a deleted key reuses its position, so the traversal
%% continues after that position.
fixtable_reinsert() ->
    T = ets:new(ets_probe_fix_reinsert, [set]),
    ets:insert(T, [{2}, {146}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 2),
    ets:insert(T, {2}),
    Next = ets:next(T, 2),
    ets:safe_fixtable(T, false),
    [Next].

%% The insert of a key after a clear reuses its old position, which is
%% before the other deleted key.
fixtable_clear_reinsert() ->
    T = ets:new(ets_probe_fix_clear_reinsert, [set]),
    ets:insert(T, [{2}, {146}]),
    ets:safe_fixtable(T, true),
    ets:internal_delete_all(T, undefined),
    ets:insert(T, {2}),
    Next = ets:next(T, 146),
    ets:safe_fixtable(T, false),
    [Next].


%% A clear skips the chain nodes that the earlier deletes kept.
fixtable_clear_after_delete() ->
    T = ets:new(ets_probe_fix_clear_after_delete, [set]),
    ets:insert(T, [{2}, {146}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 2),
    ets:internal_delete_all(T, undefined),
    Next = ets:next(T, 146),
    ets:safe_fixtable(T, false),
    [Next].

%% The insert of a bag key reuses its chain node and frees the other
%% pseudo-deleted objects of the key.
fixtable_bag_reinsert() ->
    T = ets:new(ets_probe_fix_bag_reinsert, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    ets:insert(T, [{k, 3}]),
    Fresh = ets:insert(T, [{j, 4}]),
    ets:safe_fixtable(T, false),
    [ets:lookup(T, k), Fresh, ets:lookup(T, j)].

%% A select_delete of every object of a fixed table does not join buckets.
fixtable_shrink() ->
    T = ets:new(ets_probe_fix_shrink, [set]),
    ets:insert(T, [{I} || I <- from(1, 5000)]),
    ets:safe_fixtable(T, true),
    Removed = ets:internal_select_delete(T, [{{'_'}, [], [true]}]),
    Size = ets:info(T, size),
    ets:safe_fixtable(T, false),
    [Removed, Size].


%% The insert of a duplicate_bag key reuses its chain node and frees the
%% other pseudo-deleted objects of the key.
fixtable_duplicate_bag_reinsert() ->
    T = ets:new(ets_probe_fix_duplicate_bag, [duplicate_bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    ets:insert(T, [{k, 3}]),
    Fresh = ets:insert(T, [{j, 4}]),
    ets:safe_fixtable(T, false),
    [ets:lookup(T, k), Fresh, ets:lookup(T, j)].


%% The fixed item of ets:info/2, and the safe_fixed item of an unfixed
%% table.
fixtable_info() ->
    T = ets:new(ets_probe_fix_info, [set]),
    Before = ets:info(T, fixed),
    ets:safe_fixtable(T, true),
    During = ets:info(T, fixed),
    Safe = ets:info(ets:new(ets_probe_fix_info_safe, [set]), safe_fixed),
    ets:safe_fixtable(T, false),
    After = ets:info(T, fixed),
    [Before, During, Safe, After].

%% An ordered_set that a process deletes a key from while fixed keeps the
%% ordinary delete and compaction.
fixtable_ordered_set() ->
    T = ets:new(ets_probe_fix_ordered, [ordered_set]),
    ets:insert(T, [{1, a}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, 1),
    Lookup = ets:lookup(T, 1),
    ets:safe_fixtable(T, false),
    [Lookup, ets:info(T, size)].


%% The safe_fixed details of a fixed table, which PBeam does not keep.
fixtable_safe_fixed() ->
    T = ets:new(ets_probe_fix_safe_fixed, [set]),
    ets:safe_fixtable(T, true),
    ets:info(T, safe_fixed).


%% The insert of a bag key after a delete keeps the deleted objects, so a
%% later insert compares them too.
fixtable_bag_repeat() ->
    T = ets:new(ets_probe_fix_bag_repeat, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    ets:insert(T, [{k, 3}]),
    ets:insert(T, [{k, 4}]),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Objects].

%% The select delete of one object of a bag keeps it as a pseudo-deleted
%% object of the live key.
fixtable_bag_partial() ->
    T = ets:new(ets_probe_fix_bag_partial, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    Removed = ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:insert(T, [{k, 3}]),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Removed, Objects].


%% A delete of the key of a bag with an object that a select delete kept
%% counts each slot once.
fixtable_bag_double_delete() ->
    T = ets:new(ets_probe_fix_bag_double_delete, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:delete(T, k),
    Size = ets:info(T, size),
    ets:safe_fixtable(T, false),
    [Size].

%% The insert of an object that a select delete kept restores it.
fixtable_bag_restore() ->
    T = ets:new(ets_probe_fix_bag_restore, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:insert(T, [{k, 2}]),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Objects].

%% The second delete of the key of a bag with a pseudo-deleted object
%% keeps the key gone after the unfix.
fixtable_bag_last_live() ->
    T = ets:new(ets_probe_fix_bag_last_live, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:internal_select_delete(T, [{{k, 1}, [], [true]}]),
    Before = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Before, ets:lookup(T, k)].


%% The newest live object of a bag key skips the pseudo-deleted object
%% behind it.
fixtable_bag_skip_deleted() ->
    T = ets:new(ets_probe_fix_bag_skip_deleted, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}, {k, 3}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:internal_select_delete(T, [{{k, 3}, [], [true]}]),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Objects].

%% A key whose chain node is pseudo-deleted still comes next while one of
%% its older objects is live.
fixtable_next_live_behind_node() ->
    T = ets:new(ets_probe_fix_next_live_behind_node, [bag]),
    ets:insert(T, [{1, a}, {2, a}, {2, b}, {2, c}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{2, c}, [], [true]}]),
    Next = ets:next(T, 1),
    ets:safe_fixtable(T, false),
    [Next].

%% The clear of a fixed bag also frees the live objects behind a
%% pseudo-deleted chain node.
fixtable_clear_behind_node() ->
    T = ets:new(ets_probe_fix_clear_behind_node, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:internal_delete_all(T, undefined),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Objects].

%% The insert of an object that a select delete kept restores it at the
%% head of the object list.
fixtable_bag_restore_behind_node() ->
    T = ets:new(ets_probe_fix_bag_restore_behind_node, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}, {k, 3}, {k, 4}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:insert(T, [{k, 2}]),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Objects].

%% The explicit delete of a table that the process fixed removes the
%% fixations too (`erl_db.c:3053-3061`).
fixtable_delete_table() ->
    T = ets:new(ets_probe_fix_delete_table, [set]),
    ets:insert(T, {k}),
    ets:safe_fixtable(T, true),
    [ets:delete(T)].

%% A clear of a large fixed table marks 8 buckets for each reduction, so
%% it traps and continues in a later turn (`erl_db_hash.c:3007-3051`).
fixtable_clear_large() ->
    T = ets:new(ets_probe_fix_clear_large, [set]),
    ets:insert(T, [{I} || I <- from(1, 33000)]),
    ets:safe_fixtable(T, true),
    Count = ets:internal_delete_all(T, undefined),
    Size = ets:info(T, size),
    ets:safe_fixtable(T, false),
    [Count, Size].

%% A process that uses the table during a fixed clear helps with the mark,
%% 8 buckets for each reduction (`erl_db.c:3300-3320`).
fixtable_clear_help() ->
    T = ets:new(ets_probe_fix_clear_help, [public, set]),
    ets:insert(T, [{I} || I <- from(1, 100000)]),
    ets:safe_fixtable(T, true),
    Self = self(),
    spawn(?MODULE, clear_task, [T, Self]),
    Size = wait_size(T),
    receive {cleared, Count} -> [Count, Size] end.

clear_task(T, Self) ->
    Self ! {cleared, ets:internal_delete_all(T, undefined)}.

wait_size(T) ->
    case ets:info(T, size) of
        0 -> 0;
        _ -> wait_size(T)
    end.

%% The take of a key of a fixed table keeps the first live object as a
%% pseudo-deleted object and frees the records after it, and the item
%% counter falls by each of those records (`erl_db_hash.c:2949-2995`).
fixtable_take() ->
    T = ets:new(ets_probe_fix_take, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}, {k, 3}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    Taken = ets:take(T, k),
    Size = ets:info(T, size),
    Objects = ets:lookup(T, k),
    ets:insert(T, [{j, 9}, {l, 10}]),
    Both = [ets:lookup(T, j), ets:lookup(T, l)],
    ets:safe_fixtable(T, false),
    [Taken, Size, Objects, Both].

%% A completed clear resets the item counter (`RESET_NITEMS`,
%% `erl_db_hash.c:3048-3050`).
fixtable_take_clear() ->
    T = ets:new(ets_probe_fix_take_clear, [bag]),
    ets:insert(T, [{k, 1}, {k, 2}, {k, 3}]),
    ets:safe_fixtable(T, true),
    ets:internal_select_delete(T, [{{k, 2}, [], [true]}]),
    ets:take(T, k),
    Cleared = ets:internal_delete_all(T, undefined),
    Size = ets:info(T, size),
    ets:safe_fixtable(T, false),
    [Cleared, Size].

%% The key-clash walk of `ets:insert_new/2` adds a new object for a bag
%% key whose objects are all pseudo-deleted (`erl_db_hash.c:1412-1422`).
fixtable_insert_new() ->
    T = ets:new(ets_probe_fix_insert_new, [bag]),
    ets:insert(T, [{k, 1}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    Added = ets:insert_new(T, {k, 1}),
    Objects = ets:lookup(T, k),
    Taken = ets:take(T, k),
    Size = ets:info(T, size),
    ets:safe_fixtable(T, false),
    [Added, Objects, Taken, Size].

%% A list `ets:insert_new/2` does its own membership walk and then inserts
%% as a put, so it restores the pseudo-deleted object
%% (`erl_db.c:1858`, `1943`).
fixtable_insert_new_list() ->
    T = ets:new(ets_probe_fix_insert_new_list, [bag]),
    ets:insert(T, [{k, 1}]),
    ets:safe_fixtable(T, true),
    ets:delete(T, k),
    Added = ets:insert_new(T, [{k, 2}, {k, 1}]),
    Objects = ets:lookup(T, k),
    ets:safe_fixtable(T, false),
    [Added, Objects].

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.

%% Return the error, its reason, and the error information of the first
%% stack frame, as the first frame of a BIF raise gives it.
catch_info(Fun) ->
    try Fun()
    catch error:Reason:Stack ->
            [Frame | _] = Stack,
            {error, Reason, element(4, Frame)}
    end.

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
                  %% A replacement moves the key to the new exactly equal
                  %% term, and its stored hash.
                  ets:insert(T, {1.0, replaced}),
                  ets:lookup(T, 1),
                  ets:insert_new(T, {1, other}),
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
    D = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(D, [{a, 1}, {b, 2}, {c, 3}]),
    Call = fun(F) -> Before = reductions(), F(), reductions() - Before end,
    [Call(fun() -> ets:lookup(T, a) end),
     Call(fun() -> ets:lookup(T, a) end),
     Call(fun() -> ets:insert(T, {d, 4}) end),
     Call(fun() -> ets:update_element(T, a, {2, 9}) end),
     Call(fun() -> ets:take(T, b) end),
     Call(fun() -> ets:delete(T, a) end),
     Call(fun() -> ets:internal_delete_all(T, undefined) end),
     %% `ets:delete/1` frees one object for each reduction of the tree
     %% (`do_free_tree_continue`, `erl_db_tree.c:2877-2904`).
     Call(fun() -> ets:delete(D) end)].

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

%% The first key and the first {Key, Objects} of a table (erl_db.c:1170-1211,
%% db_first_hash_common, erl_db_hash.c:1117-1145).
first() ->
    Set = ets:new(first_set, [set]),
    Bag = ets:new(first_bag, [bag]),
    Dup = ets:new(first_dup, [duplicate_bag]),
    Empty = ets:new(first_empty, []),
    true = ets:insert(Set, [{a, 1}, {b, 2}]),
    true = ets:insert(Bag, [{a, 1}, {a, 2}, {b, 3}]),
    true = ets:insert(Dup, [{a, 1}, {a, 2}, {b, 3}]),
    Invalid = catch_error(fun() -> ets:first(make_ref()) end),
    [ets:first(Set), ets:first_lookup(Set),
     ets:first(Bag), ets:first_lookup(Bag),
     ets:first(Dup), ets:first_lookup(Dup),
     ets:first(Empty), ets:first_lookup(Empty),
     Invalid, catch_error(fun() -> ets:first_lookup(not_a_table) end)].

%% The eight walk BIFs, ets:foldl/3, and ets:foldr/3 on each table type.
%% An ordered_set walks its keys in the term order, with 1 and 1.0 as one
%% key, and a step from a key that is not in the table gives the next key.
%% A hash table gives last as first and prev as next.
walk() ->
    O = ets:new(ets_probe, [ordered_set]),
    E = ets:first(O),
    true = ets:insert(O, [{1, a}, {2.5, b}, {x, c}, {{1}, d}, {1.0, e}, {-3, f}]),
    true = ets:insert(O, {"s", g}),
    true = ets:delete(O, x),
    Ordered = [E, ets:last(O), ets:first_lookup(O), ets:last_lookup(O),
               ets:first(O), ets:last(O), ets:next(O, 1), ets:next(O, 1.0),
               ets:next(O, 2), ets:prev(O, 2.5), ets:prev(O, -3),
               ets:next(O, "s"), ets:next(O, zzz), ets:prev(O, x),
               ets:next_lookup(O, 1), ets:prev_lookup(O, {9}),
               ets:prev_lookup(O, -9), ets:lookup(O, 1), ets:take(O, 1),
               ets:first(O), ets:info(O, size),
               ets:foldl(fun({K, _}, A) -> [K | A] end, [], O),
               ets:foldr(fun({K, _}, A) -> [K | A] end, [], O)],
    Empty = ets:new(ets_probe, [ordered_set]),
    End = [walk_error(B, Tab, '$end_of_table')
           || Tab <- [Empty, O], B <- [next, prev, next_lookup, prev_lookup]],
    Hash = [hash_walk(Type) || Type <- [set, bag, duplicate_bag]],
    [Ordered, End, Hash].

hash_walk(Type) ->
    T = ets:new(ets_probe, [Type]),
    E = [ets:first(T), ets:last(T), ets:first_lookup(T), ets:last_lookup(T)],
    true = ets:insert(T, [{1, a}, {2, b}, {1, c}, {k, d}]),
    F = ets:first(T),
    L = ets:last(T),
    [E, F, L, ets:last_lookup(T), ets:next(T, F), ets:prev(T, F),
     ets:next_lookup(T, F), ets:prev_lookup(T, F),
     [walk_error(B, T) || B <- [next, prev, next_lookup, prev_lookup]],
     lists:sort(ets:foldl(fun(O, A) -> [O | A] end, [], T)),
     lists:sort(ets:foldr(fun(O, A) -> [O | A] end, [], T))].

walk_error(F, T) ->
    walk_error(F, T, 9).

walk_error(F, T, K) ->
    try ets:F(T, K) catch error:R -> {error, R} end.

%% Two different alias references have no PBeam order, so an ordered_set
%% key that holds one gives a runtime error on a walk.
walk_alias(Case) ->
    O = ets:new(ets_probe, [ordered_set]),
    A = erlang:alias([]),
    B = erlang:alias([]),
    true = ets:insert(O, {A, 1}),
    case Case of
        step ->
            ets:next(O, B);
        unordered ->
            true = ets:insert(O, [{B, 2}, {1.0, 3}]),
            [ets:lookup(O, 1), ets:delete(O, A), ets:lookup(O, B)];
        unordered_first ->
            true = ets:insert(O, {B, 2}),
            ets:first(O);
        cleared ->
            true = ets:insert(O, {B, 2}),
            true = ets:delete_all_objects(O),
            true = ets:insert(O, [{2, b}, {1, a}]),
            [ets:first(O), ets:last(O)]
    end.

%% ets:select/1,2,3, ets:select_reverse/1,2,3, ets:match/2,3,
%% ets:match_object/2, ets:tab2list/1, and ets:select_delete/2 walk an
%% `ordered_set` table in key order (erl_db_tree.c:1263-2215). A bound or
%% partially bound key limits the walk to a key range (analyze_pattern,
%% erl_db_tree.c:2756-2875).
ordered_select() ->
    Keys = [3, 1.5, b, a, {1, x}, {1, y}, {2, z}, {1, x, w}, [1, 2], [1 | c],
            "ab", <<"c">>, {[1, 2], q}, 7],
    T = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(T, [{K, v} || K <- Keys]),
    Specs = [[{'$1', [], ['$1']}],
             [{{'$1', v}, [], ['$1']}],
             [{{{1, '_'}, '$1'}, [], ['$_']}],
             [{{{1, '$2'}, '_'}, [{'=/=', '$2', x}], ['$2']}],
             [{{a, '_'}, [], ['$_']}],
             [{{3.0, '_'}, [], ['$_']}],
             [{{a, '_'}, [], [a]}, {{b, '_'}, [], [b]}],
             [{{b, '_'}, [], [b]}, {{{2, '_'}, '_'}, [], [two]}],
             [{{[1 | '_'], '_'}, [], ['$_']}],
             [{{{'$1', q}, '_'}, [], ['$1']}],
             [{{{[1 | '$1'], '_'}, '_'}, [], ['$1']}],
             [{foo, [], [x]}],
             [{{nokey, '_'}, [], [x]}],
             [{{{0, '_'}, '_'}, [], [x]}],
             [{{{1, x, '_'}, '_'}, [], ['$_']}, {{1.5, '_'}, [], ['$_']}],
             []],
    [{ets:select(T, S), ets:select_reverse(T, S),
      [chain(ets:select(T, S, N), fun ets:select/1) || N <- [1, 2, 5, 100]],
      [chain(ets:select_reverse(T, S, N), fun ets:select_reverse/1) || N <- [1, 3]]}
     || S <- Specs]
    ++ [ets:match(T, {'$1', '_'}), ets:match(T, {{1, '$1'}, '$2'}),
        chain(ets:match(T, {'$1', '_'}, 4), fun ets:select/1),
        ets:match_object(T, {{'_', '_'}, '_'}), ets:tab2list(T),
        ets:select(T, [{{'$1', '_'}, [{is_atom, '$1'}], ['$1']}], 1000)]
    ++ [select_delete_count(Keys, S)
        || S <- [[{{{1, '_'}, '_'}, [], [true]}], [{{a, '_'}, [], [true]}],
                 [{{'$1', '_'}, [{is_number, '$1'}], [true]}], [{foo, [], [true]}], [{{nokey, '_'}, [], [true]}],
                 [{{a, '_'}, [], [false]}]]]
    ++ [named_select(), forged_select(), select_delete_yield()]
    ++ [select_recreated(Type) || Type <- [set, bag, duplicate_bag, ordered_set]].

%% The continuation of a named ordered_set table holds the name
%% (erl_db_tree.c:1937, 1960).
named_select() ->
    N = ets_probe_named,
    N = ets:new(N, [ordered_set, named_table]),
    true = ets:insert(N, [{K} || K <- seq(3)]),
    {First, C} = ets:select(N, [{'_', [], ['$_']}], 1),
    {Next, C2} = ets:select(C),
    true = ets:insert(N, {0}),
    {Last, _} = ets:select(C2),
    true = ets:delete(N),
    {First, element(1, C), Next, element(1, C2), Last}.

%% A continuation keeps its match program after its table is deleted, as
%% the magic binary of the program lives with the reference
%% (erl_db_util.h:617-619). The continuation of a named ordered_set table
%% holds the name, so it runs on a new table of the name
%% (erl_db_tree.c:1937, 1960). The continuation of a hash table holds the
%% table identifier, so it raises badarg (erl_db_hash.c:2229).
select_recreated(Type) ->
    N = ets_probe_recreated,
    N = ets:new(N, [Type, named_table]),
    true = ets:insert(N, [{K, K} || K <- seq(6)]),
    {_, C} = ets:select(N, [{{'$1', '_'}, [], ['$1']}], 2),
    true = ets:delete(N),
    N = ets:new(N, [Type, named_table]),
    true = ets:insert(N, [{K, K * 10} || K <- seq(8)]),
    Result = try ets:select(C) of
                 {Keys, C2} -> {Keys, element(2, C2)}
             catch Class:Reason -> {Class, Reason}
             end,
    true = ets:delete(N),
    {Type, Result}.

%% A continuation with a known program and a bad Got or Matches raises
%% badarg (erl_db_tree.c:1287-1302).
forged_select() ->
    T = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(T, [{K} || K <- seq(3)]),
    {_, C} = ets:select(T, [{'_', [], ['$_']}], 1),
    Try = fun(X) -> try ets:select(X) catch Class:Reason -> {Class, Reason} end end,
    [Try(setelement(8, C, foo)), Try(setelement(6, C, foo))].

%% A table deleted while a select delete traps raises badarg in
%% ets:select_delete_trap/1 (erl_db.c:3387-3427, erl_db_tree.c:2198-2210).
%% The probe drops the stack trace: PBeam gives `[]` after a trap (backlog
%% "Give the stack trace of a BIF error after a trap").
select_delete_yield() ->
    T = ets:new(ets_probe, [ordered_set, public]),
    true = ets:insert(T, [{K} || K <- seq(20000)]),
    Self = self(),
    P = spawn(?MODULE, select_delete_worker, [T, Self]),
    P ! go,
    Wait = fun W() ->
                   case ets:info(T, size) of
                       20000 -> erlang:yield(), W();
                       Size -> Size > 0
                   end
           end,
    Current = Wait(),
    true = ets:delete(T),
    receive {done, R} -> {Current, R} end.

select_delete_worker(T, Parent) ->
    receive go -> ok end,
    Parent ! {done, try ets:select_delete(T, [{{'$1'}, [{'>', '$1', 0}], [true]}])
                    catch Class:Reason -> {Class, Reason}
                    end}.

select_delete_count(Keys, Spec) ->
    T = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(T, [{K, v} || K <- Keys]),
    Count = ets:select_delete(T, Spec),
    {Count, ets:tab2list(T)}.

%% Follow a select with a limit to its end, and show each continuation
%% without its table and program references.
chain('$end_of_table', _) -> ['$end_of_table'];
chain({Matches, '$end_of_table'}, _) -> [{Matches, '$end_of_table'}];
chain({Matches, C}, Next) -> [{Matches, cont(C)} | chain(Next(C), Next)].

cont(C) when tuple_size(C) =:= 8 ->
    {element(2, C), element(3, C), element(4, C), element(6, C), element(7, C), element(8, C)};
cont(C) -> {element(2, C), element(3, C), element(5, C), element(6, C)}.

%% The traps and reductions of a walk of an `ordered_set` table with more
%% than 1000 objects (erl_db_tree.c:1309-1417, 1537-1561, 1199-1245).
ordered_select_large() ->
    T = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(T, [{{I div 1000, I}, I} || I <- seq(4500)]),
    Call = fun(F) ->
                   Before = reductions(),
                   R = F(),
                   Reductions = reductions() - Before,
                   {Reductions, summary(R)}
           end,
    All = [{'_', [], ['$_']}],
    Range = [{{{2, '_'}, '_'}, [], ['$_']}],
    Odd = [{{'_', '$1'}, [{'=:=', {'rem', '$1', 2}, 1}], ['$1']}],
    None = [{{'_', '$1'}, [{'<', '$1', 0}], ['$1']}],
    %% A walk that builds a large result also collects garbage on ERTS, which
    %% costs reductions that PBeam does not charge (row "Garbage collection
    %% reductions"), so these calls show only their result.
    Result = fun(F) -> summary(F()) end,
    [Result(fun() -> ets:select(T, All) end),
     Call(fun() -> ets:select_reverse(T, All) end),
     Result(fun() -> ets:select(T, Range) end),
     Call(fun() -> ets:select_reverse(T, Range) end),
     Call(fun() -> ets:select(T, Odd) end),
     Result(fun() -> ets:select(T, All, 1500) end),
     Call(fun() -> ets:select(T, All, 999) end),
     Result(fun() -> ets:select(T, All, 1000) end),
     Call(fun() -> ets:select(T, None) end),
     Call(fun() -> ets:select(T, None, 10) end),
     Call(fun() -> ets:select_reverse(T, None) end),
     Call(fun() -> ets:select_reverse(T, Range, 1200) end),
     Call(fun() -> ets:select(T, Odd, 2000) end),
     Call(fun() -> ets:select(T, [{{{3, 3500}, '_'}, [], ['$_']}]) end),
     Call(fun() -> ets:select(T, [{{{3, 3500}, '_'}, [], ['$_']}], 1) end),
     Result(fun() -> ets:select(T, All, 5000) end),
     Result(fun() -> chain_summary(ets:select(T, Range, 600)) end),
     Result(fun() -> chain_summary(ets:select_reverse(T, All, 2000)) end),
     Call(fun() -> ets:match(T, {{1, '$1'}, '_'}) end),
     Call(fun() -> ets:select_delete(T, [{{{1, '_'}, '$1'}, [{'<', '$1', 1500}], [true]}]) end),
     Call(fun() -> ets:select_delete(T, [{{'_', '$1'}, [{'>', '$1', 4000}], [true]}]) end),
     Call(fun() -> ets:select_delete(T, [{{{0, 5}, '_'}, [], [true]}]) end),
     Call(fun() -> ets:select_delete(T, [{{{3, '_'}, '$1'}, [{'<', '$1', 0}], [true]}]) end),
     Call(fun() -> ets:info(T, size) end),
     Result(fun() -> U = ets:new(ets_probe, [ordered_set]),
                     true = ets:insert(U, [{I} || I <- seq(41000)]),
                     element(1, ets:select(U, [{'_', [], ['$_']}], 50000))
            end)].

summary(L) when is_list(L) -> {length(L), erlang:phash2(L)};
summary({L, C}) when is_list(L) -> {length(L), erlang:phash2(L), cont_summary(C)};
summary(X) -> X.

cont_summary('$end_of_table') -> '$end_of_table';
cont_summary(C) -> cont(setelement(6, C, length(element(6, C)))).

chain_summary('$end_of_table') -> [];
chain_summary({L, '$end_of_table'}) -> [length(L)];
chain_summary({L, C}) -> [length(L) | chain_summary(ets:select(C))].

%% ets:select_reverse/1,2,3 on a hash table give the ets:select/1,2,3
%% result (erl_db_hash.c:2308-2318).
select_reverse() ->
    [begin
         T = ets:new(ets_probe, [Type]),
         true = ets:insert(T, [{K, V} || K <- [a, b, c, d], V <- [1, 2]]),
         S = [{{'$1', '$2'}, [], [{{'$2', '$1'}}]}],
         {Type, ets:select_reverse(T, S) =:= ets:select(T, S),
          element(1, ets:select_reverse(T, S, 3)),
          element(1, ets:select_reverse(element(2, ets:select_reverse(T, S, 3)))),
          ets:select_reverse('$end_of_table')}
     end || Type <- [set, bag, duplicate_bag, ordered_set]]
    ++ [caught(F) || F <- [fun() -> ets:select_reverse(foo, []) end,
                           fun() -> ets:select_reverse(foo, [], 1) end,
                           fun() -> ets:select_reverse(ets:new(ets_probe, [ordered_set]), [], 0) end,
                           fun() -> ets:select_reverse({}) end,
                           fun() -> ets:select_reverse(x) end,
                           fun() -> ets:select(forged(fun(R) -> {k, [], 1, R, [], 0, 0} end)) end,
                           fun() -> ets:select(forged(fun(R) -> {k, [], x, R, [], 0, 0} end)) end,
                           fun() -> ets:select(forged(fun(R) -> {k, [], 1, R, x, 0, 0} end)) end,
                           fun() -> ets:select(forged(fun(_) -> {k} end)) end,
                           fun() -> ets:select({ets:new(ets_probe, []), k}) end,
                           fun select_delete_gone/0]].

caught(F) ->
    try F() catch Class:Reason -> {Class, Reason} end.

%% A continuation of an `ordered_set` table whose fields are not valid
%% raises badarg (erl_db_tree.c:1287-1302).
forged(Fields) ->
    T = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(T, {k}),
    list_to_tuple([T | tuple_to_list(Fields(make_ref()))]).

%% A process that deletes the table while ets:select_delete/2 traps makes
%% the next turn raise badarg (erl_db.c:3387-3427).
select_delete_gone() ->
    T = ets:new(ets_probe, [ordered_set, public]),
    true = ets:insert(T, [{I} || I <- seq(20000)]),
    spawn(ets, delete, [T]),
    try ets:select_delete(T, [{{'$1'}, [{'<', '$1', 0}], [true]}])
    catch error:badarg -> badarg
    end.

%% Delete objects that are exactly equal to an object (ets_delete_object_2,
%% erl_db.c:3355-3382; db_erase_object_hash, erl_db_hash.c:1669-1730;
%% db_erase_object_tree_common, erl_db_tree.c:1120-1131).
delete_object() ->
    S = ets:new(ets_probe, []),
    true = ets:insert(S, [{a, 1}, {b, 2}]),
    O = ets:new(ets_probe, [ordered_set]),
    true = ets:insert(O, [{1, a}, {2, b}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(B, [{a, 1}, {a, 2}, {a, 3}, {b, 1}]),
    D = ets:new(ets_probe, [duplicate_bag]),
    true = ets:insert(D, [{a, 1}, {a, 2}, {a, 1}, {a, 1}, {b, 1}]),
    K = ets:new(ets_probe, [{keypos, 2}]),
    true = ets:insert(K, {x, k, y}),
    Protected = start([protected]),
    Public = start([public]),
    true = ets:insert(tid(Public), {k, v}),
    R = [ets:delete_object(S, {a, 2}), ets:lookup(S, a),
         ets:delete_object(S, {a, 1}), ets:lookup(S, a),
         ets:delete_object(S, {missing, 1}), ets:lookup(S, b),
         ets:delete_object(S, {b}), ets:lookup(S, b),
         ets:delete_object(O, {1.0, a}), ets:lookup(O, 1),
         ets:delete_object(O, {1, a}), ets:lookup(O, 1), ets:first(O),
         ets:delete_object(B, {a, 2}), ets:lookup(B, a),
         ets:delete_object(B, {a, 1}), ets:lookup(B, a),
         ets:delete_object(B, {a, 1.0}), ets:lookup(B, a),
         ets:delete_object(B, {missing, 1}), ets:lookup(B, b),
         ets:delete_object(D, {a, 1}), ets:lookup(D, a),
         ets:delete_object(D, {a, 2}), ets:lookup(D, a), ets:lookup(D, b),
         ets:delete_object(K, {x, k, z}), ets:lookup(K, k),
         ets:delete_object(K, {x, k, y}), ets:lookup(K, k),
         catch_error(fun() -> ets:delete_object(K, {x}) end),
         catch_error(fun() -> ets:delete_object(S, [a, 1]) end),
         catch_error(fun() -> ets:delete_object(S, b) end),
         catch_error(fun() -> ets:delete_object(make_ref(), {a, 1}) end),
         catch_error(fun() -> ets:delete_object(ets_probe_missing, {a}) end),
         catch_error(fun() -> ets:delete_object(tid(Protected), {k, v}) end),
         catch_error(fun() -> ets:delete_object(tid(Protected), bad) end),
         ets:delete_object(tid(Public), {k, v}), ets:lookup(tid(Public), k),
         delete_object_fixed(bag), delete_object_fixed(duplicate_bag),
         delete_object_fixed(set), delete_object_fixed(ordered_set),
         delete_object_fixed_copies(),
         delete_object_fixed_reinsert(bag),
         delete_object_fixed_reinsert(duplicate_bag)],
    stop(Protected),
    stop(Public),
    R.

%% Delete objects from a fixed table, then walk it and read it after the
%% release. A select delete first leaves pseudo-deleted objects inside the
%% chains (erl_db_hash.c:1695-1704, 2622-2653).
delete_object_fixed(Type) ->
    T = ets:new(ets_probe, [Type]),
    Objects = [{I rem 7, I rem 3} || I <- lists:seq(1, 40)],
    true = ets:insert(T, Objects),
    true = ets:safe_fixtable(T, true),
    First = ets:first(T),
    Selected = ets:select_delete(T, [{{'_', 1}, [], [true]}]),
    Deleted = [ets:delete_object(T, {I, J}) || I <- [First, 2, 4], J <- [0, 2]],
    Walk = delete_object_walk(T, First, []),
    true = ets:safe_fixtable(T, false),
    true = ets:insert(T, {First, 9}),
    {Selected, Deleted, Walk, ets:info(T, size),
     lists:sort(ets:select(T, [{'_', [], ['$_']}])), ets:first(T)}.

%% Free a later live object of a fixed table, delete the last one, and
%% insert the key again while the table is fixed.
delete_object_fixed_reinsert(Type) ->
    T = ets:new(ets_probe, [Type]),
    true = ets:insert(T, [{k, 1}, {k, 2}, {k, 3}]),
    true = ets:safe_fixtable(T, true),
    true = ets:delete_object(T, {k, 3}),
    true = ets:delete_object(T, {k, 1}),
    true = ets:delete_object(T, {k, 2}),
    true = ets:insert(T, {k, 9}),
    Fixed = ets:lookup(T, k),
    true = ets:safe_fixtable(T, false),
    {Fixed, ets:lookup(T, k), ets:info(T, size)}.

delete_object_walk(_T, '$end_of_table', Keys) ->
    lists:reverse(Keys);
delete_object_walk(T, Key, Keys) ->
    delete_object_walk(T, ets:next(T, Key), [Key | Keys]).

%% Delete two copies of the newest object of a key in a fixed duplicate_bag
%% table: the first stays pseudo-deleted and the second goes.
delete_object_fixed_copies() ->
    T = ets:new(ets_probe, [duplicate_bag]),
    true = ets:insert(T, [{k, 1}, {k, 2}, {k, 2}]),
    true = ets:safe_fixtable(T, true),
    true = ets:delete_object(T, {k, 2}),
    Fixed = {ets:lookup(T, k), ets:first(T), ets:next(T, k)},
    true = ets:safe_fixtable(T, false),
    {Fixed, ets:lookup(T, k), ets:info(T, size)}.

%% The exit of an owner gives each table to its live heir, which keeps the
%% heir and gets {'ETS-TRANSFER', Table, Owner, Data} before the 'DOWN'
%% message of the owner (erl_db.c:5077-5081, 4865-4933; erl_process.c
%% 14360-14368). {heir, Pid} sends no message (erl_db.c:2670-2673). A heir
%% that is dead at ets:new/2 or at the exit, and the owner as heir, give
%% no transfer. The exit of the heir then deletes the table, because the
%% heir is the owner (erl_db.c:5077-5078).
heir_transfer() ->
    Self = self(),
    Heir = spawn(fun() -> heir_watch(Self) end),
    Late = spawn(fun() -> receive stop -> ok end end),
    Dead = dead(),
    Owner = spawn(fun() ->
                          T1 = ets:new(ets_probe, [{heir, Heir, {data, 1}}]),
                          T2 = ets:new(ets_probe_heir2, [named_table, {heir, Heir}]),
                          T3 = ets:new(ets_probe, [{heir, Dead, x}]),
                          T4 = ets:new(ets_probe, [{heir, self(), x}]),
                          T5 = ets:new(ets_probe, [{heir, Late, x}]),
                          T6 = ets:new(ets_probe_heir6, [named_table, {heir, Heir, named}]),
                          Self ! {tables, [T1, T2, T3, T4, T5, T6]},
                          receive stop -> ok end
                  end),
    Tables = receive {tables, Ts} -> Ts end,
    Before = [ets:info(T, heir) =:= Heir || T <- Tables],
    LateRef = monitor(process, Late),
    Late ! stop,
    receive {'DOWN', LateRef, _, _, _} -> ok end,
    Heir ! {watch, Owner},
    receive watching -> ok end,
    Owner ! stop,
    Seen = receive {seen, S} -> S end,
    [T1, _, _, _, _, T6] = Tables,
    Messages = [case M of
                    {'ETS-TRANSFER', T, From, Data} ->
                        {transfer, T =:= T1 orelse T, From =:= Owner, Data};
                    {'DOWN', _, process, From, Reason} -> {down, From =:= Owner, Reason}
                end || M <- Seen],
    After = [{ets:info(T, owner) =:= Heir, ets:info(T, heir) =:= Heir} || T <- Tables],
    HeirRef = monitor(process, Heir),
    Heir ! stop,
    receive {'DOWN', HeirRef, _, _, _} -> ok end,
    Gone = [ets:info(T, owner) || T <- [T1, T6]],
    {Before, Messages, After, Gone}.

heir_watch(Parent) ->
    receive {watch, Owner} -> ok end,
    Ref = monitor(process, Owner),
    Parent ! watching,
    Parent ! {seen, heir_collect(Ref, [])},
    receive stop -> ok end.

heir_collect(Ref, Seen) ->
    receive
        {'DOWN', Ref, _, _, _} = Down -> lists:reverse([Down | Seen]);
        Message -> heir_collect(Ref, [Message | Seen])
    end.

%% ets:give_away/3 (erl_db.c:3087-3144): the result, the message, the owner
%% and the heir after the call, and each badarg with its cause in the ERTS
%% order.
give_away() ->
    Self = self(),
    Receiver = spawn(fun() -> give_receiver(Self) end),
    Dead = dead(),
    T = ets:new(ets_probe, [{heir, Dead, x}]),
    N = ets:new(ets_probe_gift, [named_table, public, {heir, self(), h}]),
    true = ets:insert(N, {k, 1}),
    {Other, OtherTable} = start([public]),
    {_, Protected} = start([]),
    Errors = [give_cause(F) || F <- [fun() -> ets:give_away(T, self(), x) end,
                                     fun() -> ets:give_away(T, Dead, x) end,
                                     fun() -> ets:give_away(T, not_a_pid, x) end,
                                     fun() -> ets:give_away(OtherTable, self(), x) end,
                                     fun() -> ets:give_away(OtherTable, Dead, x) end,
                                     fun() -> ets:give_away(Protected, self(), x) end,
                                     fun() -> ets:give_away(no_table, Dead, x) end]],
    R1 = ets:give_away(T, Receiver, {gift, 1}),
    R2 = ets:give_away(N, Receiver, gift),
    Got = receive {got, G} -> G end,
    Seen = [{Table =:= T orelse Table, From =:= Self, Data}
            || {'ETS-TRANSFER', Table, From, Data} <- Got],
    State = [{ets:info(X, owner) =:= Receiver, ets:info(X, heir) =:= Self} || X <- [T, N]],
    Lost = give_cause(fun() -> ets:give_away(N, Receiver, again) end),
    Receiver ! {back, N},
    Back = receive {back, Result, Message} -> {Result, Message} end,
    Mine = ets:info(N, owner) =:= Self,
    _ = stop({Other, x}),
    {Errors, R1, R2, Seen, State, Lost, Back, Mine, ets:lookup(N, k)}.

give_receiver(Parent) ->
    M1 = receive {'ETS-TRANSFER', _, _, _} = A -> A end,
    M2 = receive {'ETS-TRANSFER', _, _, _} = B -> B end,
    Parent ! {got, [M1, M2]},
    receive {back, N} -> ok end,
    Result = ets:give_away(N, Parent, back),
    Message = receive {'ETS-TRANSFER', Table, From, Data} -> {Table, From =:= self(), Data}
              after 0 -> none end,
    Parent ! {back, Result, Message},
    receive stop -> ok end.

%% ets:setopts/2 (erl_db.c:3146-3229): the result, ets:info/2 after each
%% call, the heir that set_heir drops (erl_db.c:5319-5337), and each badarg
%% with its cause.
setopts() ->
    Self = self(),
    Dead = dead(),
    T = ets:new(ets_probe, []),
    Info = fun() -> {ets:info(T, protection), ets:info(T, heir) =:= Self} end,
    R = [{ets:setopts(T, {heir, Self, data}), Info()},
         {ets:setopts(T, [{heir, none}]), Info()},
         {ets:setopts(T, {heir, Self}), Info()},
         {ets:setopts(T, {protection, public}), Info()},
         {ets:setopts(T, [{protection, private}, {heir, none}]), Info()},
         {ets:setopts(T, []), Info()},
         {ets:setopts(T, [{heir, Dead, x}, {protection, protected}]), Info(), ets:info(T, heir)}],
    {Other, OtherTable} = start([public]),
    {_, Protected} = start([]),
    Errors = [give_cause(fun() -> ets:setopts(Tab, Opts) end)
              || {Tab, Opts} <- [{OtherTable, {heir, none}},
                                 {OtherTable, {protection, bad}},
                                 {Protected, {heir, none}},
                                 {no_table, {heir, none}},
                                 {1, {heir, none}},
                                 {T, {heir, none, x}},
                                 {T, {heir, Self, x, y}},
                                 {T, {heir, not_a_pid}},
                                 {T, {heir}},
                                 {T, [{heir, none}, {heir, none}]},
                                 {T, [{protection, public}, {protection, public}]},
                                 {T, {protection, public, x}},
                                 {T, {protection, none}},
                                 {T, {keypos, 2}},
                                 {T, heir},
                                 {T, [{heir, none} | tail]},
                                 {T, [{protection, public}, bad]}]],
    After = Info(),
    _ = stop({Other, x}),
    {R, Errors, After}.

%% Return the reason and the error_info cause of the first stack frame.
give_cause(Fun) ->
    try Fun()
    catch error:Reason:Stack ->
            [{_, _, _, Info} | _] = Stack,
            {Reason, maps:get(cause, proplists:get_value(error_info, Info), none)}
    end.

%% ets:update_counter/3,4 (erl_db.c:1494-1730).
update_counter() ->
    T = ets:new(ets_probe, []),
    O = ets:new(ets_probe, [ordered_set, {keypos, 2}]),
    B = ets:new(ets_probe, [bag]),
    true = ets:insert(T, [{a, 1, 2, x}, {b}, {1, 0}]),
    true = ets:insert(O, [{0, a, 10}, {0, 1, 5}]),
    true = ets:insert(B, [{a, 1}]),
    Big = 1 bsl 62,
    [ets:update_counter(T, a, 1),
     ets:update_counter(T, a, {3, -1}),
     ets:update_counter(T, a, [{2, Big}, {2, Big}, {3, -5, 0, 7}, {3, 1, 7, -1}]),
     ets:lookup(T, a),
     ets:update_counter(T, a, [{2, -Big}, {2, -Big}]),
     ets:update_counter(T, a, [{2, 10, 5, 2}, {3, -1, 0, 9}, {3, -1, -1, 4}]),
     ets:update_counter(T, a, {2, 1, Big, Big}),
     ets:update_counter(T, a, []),
     ets:lookup(T, a),
     ets:update_counter(T, 1.0, 1, {x, 0}),
     ets:lookup(T, 1.0),
     ets:update_counter(T, z, {2, 1}, {q, 5}),
     ets:update_counter(T, y, [], {q, 5, 6}),
     ets:update_counter(T, w, [{3, 2}, {2, 1}], {q, 5, 6}),
     lists:sort(ets:tab2list(T)),
     ets:update_counter(O, a, 1),
     ets:update_counter(O, 1.0, [{3, 2}, {1, 4}]),
     ets:update_counter(O, c, {3, 1}, {d, e, 0}),
     ets:tab2list(O),
     catch_error(fun() -> ets:update_counter(B, a, 1) end),
     catch_error(fun() -> ets:update_counter(T, a, {1, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, {5, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, {0, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, {4, 1}) end),
     catch_error(fun() -> ets:update_counter(T, b, 1) end),
     catch_error(fun() -> ets:update_counter(T, missing, 1) end),
     catch_error(fun() -> ets:update_counter(T, a, foo) end),
     catch_error(fun() -> ets:update_counter(T, a, [{2, 1} | bad]) end),
     catch_error(fun() -> ets:update_counter(T, a, [{2, 1}, bad]) end),
     catch_error(fun() -> ets:update_counter(T, a, {2, a}) end),
     catch_error(fun() -> ets:update_counter(T, a, {2, 1, a, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, {2, 1, 1, a}) end),
     catch_error(fun() -> ets:update_counter(T, a, {2, 1, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, {1 bsl 60, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, {2.0, 1}) end),
     catch_error(fun() -> ets:update_counter(T, a, [{2, 1}, {1, 1}]) end),
     catch_error(fun() -> ets:update_counter(T, v, {2, a}, {q, 0}) end),
     catch_error(fun() -> ets:update_counter(T, v, 1, bad) end),
     catch_error(fun() -> ets:update_counter(O, v, 1, {v}) end),
     catch_error(fun() -> ets:update_counter(T, v, 1, {v}) end),
     ets:lookup(T, v),
     ets:lookup(T, a),
     catch_error(fun() -> ets:update_counter(ets_probe_missing, a, 1) end),
     begin
         {PrivateOwner, Private, _} = private_table(),
         Result = [catch_error(fun() -> ets:update_counter(Private, a, 1) end),
                   catch_error(fun() -> ets:update_counter(Private, a, 1, bad) end)],
         PrivateOwner ! stop,
         Result
     end].
