-module(ets_tables).
-export([owner/0, user/0, fixer/0, deleter/0, inserter/0, helper/0, testator/0, heir/0, walker/0, walk_peer/0, walk_orders/0, reference/0]).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun owner/0, fun user/0),
     pair(fun fixer/0, fun deleter/0),
     pair(fun inserter/0, fun helper/0),
     pair(fun testator/0, fun heir/0),
     pair(fun walker/0, fun walk_peer/0)].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

%% Each root returns ok when its result is the expected result, and else
%% {unexpected, Result}.
owner() -> check(owner_run(), ok).
user() ->
    check(user_run(),
          {[[{k, 1}], true, [{u, 2}], [{k, 1}], badarg, badarg, true,
            [{auto, true}, {true, false}, {true, true}, {false, false},
             {false, false}, {{debug_hash_fixed_number_of_locks, 64}, true},
             {{debug_hash_fixed_number_of_locks, 64}, true},
             {{debug_hash_fixed_number_of_locks, 256}, true},
             {true, true}, {true, true}, {true, false}]],
           [undefined, undefined, undefined, undefined]}).
fixer() -> check(fixer_run(), {true, undefined}).
deleter() -> check(deleter_run(), ok).
inserter() -> check(inserter_run(), {true, 100000}).
helper() -> check(helper_run(), ok).

testator() -> check(testator_run(), true).
heir() ->
    check(heir_run(),
          {[{'ETS-TRANSFER', ets_gift, true, gift},
            {'ETS-TRANSFER', ets_inherit, true, legacy},
            {'DOWN', true, normal}],
           true, [{k, 1}]}).

check(Expected, Expected) -> ok;
check(Result, _) -> {unexpected, Result}.

wait_for(Name) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name) end;
        Pid -> Pid
    end.

%% The owner makes a table of each access mode, lets the user of the other
%% scheduler read and write them, and exits, which deletes the tables.
owner_run() ->
    ets:new(ets_public, [named_table, public]),
    ets:new(ets_protected, [named_table, protected]),
    ets:new(ets_private, [named_table, private]),
    [true = ets:insert(T, {k, 1}) || T <- [ets_public, ets_protected, ets_private]],
    register(ets_owner, self()),
    receive {done, _} -> ok after 60000 -> timeout end.

user_run() ->
    Owner = wait_for(ets_owner),
    Seen = [ets:lookup(ets_public, k),
            ets:insert(ets_public, {u, 2}),
            ets:lookup(ets_public, u),
            ets:lookup(ets_protected, k),
            error_of(fun() -> ets:insert(ets_protected, {u, 2}) end),
            error_of(fun() -> ets:lookup(ets_private, k) end),
            ets:info(ets_public, owner) =:= Owner,
            [options(O) || O <- [[{write_concurrency, auto}],
                                 [{write_concurrency, true}],
                                 [bag, {write_concurrency, true}, {decentralized_counters, true}],
                                 [private, {write_concurrency, true}],
                                 [{write_concurrency, true}, {write_concurrency, false}],
                                 [{write_concurrency, {debug_hash_fixed_number_of_locks, 64}}],
                                 [{write_concurrency, {debug_hash_fixed_number_of_locks, 2}}],
                                 [{write_concurrency, {debug_hash_fixed_number_of_locks, 300}}],
                                 [ordered_set, {write_concurrency, true}],
                                 [{write_concurrency, true}, ordered_set],
                                 [ordered_set, {write_concurrency, true}, {decentralized_counters, false}]]]],
    Monitor = monitor(process, Owner),
    Owner ! {done, self()},
    receive {'DOWN', Monitor, process, Owner, _} -> ok after 60000 -> timeout end,
    Gone = [ets:info(ets_public), ets:whereis(ets_public),
            ets:whereis(ets_protected), ets:whereis(ets_private)],
    {Seen, Gone}.

options(Options) ->
    T = ets:new(options_probe, Options),
    Result = {ets:info(T, write_concurrency), ets:info(T, decentralized_counters)},
    ets:delete(T),
    Result.

error_of(Fun) ->
    try Fun() catch error:Reason -> Reason end.

%% The fixer fixes a table of the deleter of the other scheduler and waits;
%% the delete of the table leaves a fixation cleanup for the fixer.
fixer_run() ->
    Deleter = wait_for(ets_deleter),
    Fixed = ets:safe_fixtable(ets_fixed, true),
    Deleter ! {fixed, self()},
    receive deleted -> ok after 60000 -> timeout end,
    {Fixed, ets:whereis(ets_fixed)}.

deleter_run() ->
    ets:new(ets_fixed, [named_table, public]),
    register(ets_deleter, self()),
    receive {fixed, Fixer} -> ok after 60000 -> Fixer = timeout end,
    true = ets:delete(ets_fixed),
    Fixer ! deleted,
    ok.

%% The inserter starts a list insert that traps, and the helper of the
%% other scheduler uses the busy table, which helps the insert
%% (erl_db.c:769-798). The inserter still gets the result of its insert.
inserter_run() ->
    ets:new(ets_busy, [named_table, public]),
    register(ets_inserter, self()),
    Result = ets:insert(ets_busy, [{I} || I <- seq(100000, [])]),
    Size = ets:info(ets_busy, size),
    receive done -> ok after 60000 -> timeout end,
    {Result, Size}.

helper_run() ->
    Inserter = wait_for(ets_inserter),
    _ = [ets:member(ets_busy, I) || I <- seq(2000, [])],
    Inserter ! done,
    ok.

seq(0, Acc) -> Acc;
seq(N, Acc) -> seq(N - 1, [N | Acc]).

%% The testator gives one table to the heir of the other scheduler with
%% ets:give_away/3, and the heir gets the other table at the exit of the
%% testator, with each ETS-TRANSFER message before the 'DOWN' message.
testator_run() ->
    register(ets_testator, self()),
    Heir = wait_for(ets_heir),
    ets:new(ets_inherit, [named_table, {heir, Heir, legacy}]),
    true = ets:insert(ets_inherit, {k, 1}),
    ets:new(ets_gift, [named_table]),
    receive go -> ok after 60000 -> timeout end,
    ets:give_away(ets_gift, Heir, gift).

heir_run() ->
    register(ets_heir, self()),
    Testator = wait_for(ets_testator),
    Monitor = monitor(process, Testator),
    Testator ! go,
    Seen = [case M of
                {'ETS-TRANSFER', T, From, Data} -> {'ETS-TRANSFER', T, From =:= Testator, Data};
                {'DOWN', Monitor, process, From, Reason} -> {'DOWN', From =:= Testator, Reason}
            end || M <- collect(Monitor, [])],
    {Seen, ets:info(ets_inherit, owner) =:= self(), ets:lookup(ets_inherit, k)}.

collect(Monitor, Seen) ->
    receive
        {'DOWN', Monitor, _, _, _} = Down -> lists:reverse([Down | Seen]);
        Message -> collect(Monitor, [Message | Seen])
    after 60000 -> lists:reverse(Seen)
    end.

%% The walker reads hash tables in each traversal order: ets:first/1 and
%% ets:next/2, ets:tab2list/1, a select in chunks with the slot of each
%% continuation, ets:match/2, and ets:select_delete/2. A fine-locked
%% table visits the buckets of one lock before the next lock
%% (erl_db_hash.c:539-555). The result holds the length and the hash of each
%% order, so it compares the full order.
walker() -> check(walk_orders(),
          {[[{600,111998191},{600,68768075},{12,10203746},{600,112233409},{300,41177052},{300,90382645}],
       [{3000,76394115},{3000,70110769},{60,134062952},{3000,33791159},{1500,85897492},{1500,100821820}],
       [{600,53571465},{900,47043317},{18,128298444},{600,81593921},{450,77142101},{450,107509323}],
       [{600,53571465},{900,47043317},{18,128298444},{600,81593921},{450,77142101},{450,107509323}],
       [{600,47226379},{600,82042838},{12,71104751},{600,39783482},{300,41177052},{300,121082964}],
       [{600,40107970},{600,69911001},{12,17748742},{600,33547868},{300,41177052},{300,16760541}],
       [{600,69985610},{600,28137847},{12,112590183},{600,57260759},{300,41177052},{300,90197391}],
       [{600,82151543},{600,91100552},{12,71605639},{600,71291308},{300,41177052},{300,59002820}],
       [{600,82151543},{600,91100552},{12,71605639},{600,71291308},{300,41177052},{300,59002820}],
       [{3,25788620},{3,132628895},{1,33522079},{3,81555660},{1,2614250},{2,113405851}]],
      {1000,124757479}, {1000,62089682}}).
walk_peer() -> ok.

walk_orders() ->
    Fine = {write_concurrency, true},
    Central = {decentralized_counters, false},
    Locks = fun(N) -> {write_concurrency, {debug_hash_fixed_number_of_locks, N}} end,
    Walks = [walk([set, public, Fine], 600),
             walk([set, public, Fine], 3000),
             walk([bag, public, Fine], 600),
             walk([duplicate_bag, Fine], 600),
             walk([Locks(128), Central], 600),
             walk([Locks(256), Central], 600),
             walk([Locks(1024), Central], 600),
             walk([private, Fine], 600),
             walk([set], 600),
             walk([{write_concurrency, auto}], 3)],
    {Walks, shrunk([Locks(4096), Central]), shrunk([Fine])}.

walk(Options, Count) ->
    T = ets:new(walk_probe, Options),
    Bag = lists:member(bag, Options) orelse lists:member(duplicate_bag, Options),
    true = ets:insert(T, [{K, a} || K <- seq(Count, [])]),
    Bag andalso ets:insert(T, [{K, b} || K <- seq(Count div 2, [])]),
    All = [{'$1', [], ['$1']}],
    Even = [{{'$1', '_'}, [{'=:=', {'rem', '$1', 2}, 0}], [true]}],
    Orders = [first_next(T, ets:first(T), []),
              ets:tab2list(T),
              chunks(ets:select(T, All, 50), []),
              ets:match(T, {'$1', a}),
              ets:select_delete(T, Even),
              ets:tab2list(T)],
    true = ets:delete(T),
    [{length_of(O), erlang:phash2(O)} || O <- Orders].

shrunk(Options) ->
    T = ets:new(walk_probe, Options),
    true = ets:insert(T, [{K} || K <- seq(5000, [])]),
    _ = [ets:delete(T, K) || K <- seq(4000, [])],
    Order = ets:tab2list(T),
    true = ets:delete(T),
    {length(Order), erlang:phash2(Order)}.

length_of(O) when is_list(O) -> length(O);
length_of(O) -> O.

first_next(_, '$end_of_table', Acc) -> lists:reverse(Acc);
first_next(T, K, Acc) -> first_next(T, ets:next(T, K), [K | Acc]).

chunks('$end_of_table', Acc) -> lists:reverse(Acc);
chunks({Objects, '$end_of_table'}, Acc) -> lists:reverse([Objects | Acc]);
chunks({Objects, Cont}, Acc) ->
    chunks(ets:select(Cont), [{element(2, Cont), Objects} | Acc]).
