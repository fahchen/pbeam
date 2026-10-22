-module(ets_tables).
-export([owner/0, user/0, fixer/0, deleter/0, inserter/0, helper/0, reference/0]).

%% Run each pair of roots on the pinned ERTS, as PBeam runs the two roots of
%% a pair on two scheduler threads, and return the results of each pair.
reference() ->
    [pair(fun owner/0, fun user/0),
     pair(fun fixer/0, fun deleter/0),
     pair(fun inserter/0, fun helper/0)].

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
             {true, true}, {true, true}, {true, false}]],
           [undefined, undefined, undefined, undefined]}).
fixer() -> check(fixer_run(), {true, undefined}).
deleter() -> check(deleter_run(), ok).
inserter() -> check(inserter_run(), {true, 100000}).
helper() -> check(helper_run(), ok).

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
