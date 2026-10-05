-module(ets_probe).
-export([new/0, owner_exit/0, insert/0, lookup/0, access/0, main/0, env/0,
         unsupported/1, own/2]).

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
%% the DOWN message (erl_process.c:14360-14368, erl_db.c:5031-5093).
owner_exit() ->
    Owner = start([named_table]),
    [catch_error(fun() -> ets:new(ets_probe_owned, [named_table]) end),
     stop(Owner),
     ets:new(ets_probe_owned, [named_table])].

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

%% PBeam has no application_controller, so main/0 makes ac_tab with the
%% options of application_controller.erl:544-545 in an owner process.
main() ->
    _ = start(ac_tab),
    env().

env() ->
    [application:get_env(kernel, standard_io_encoding, default),
     ets:insert(ac_tab, {{env, ets_probe, key}, value}),
     application:get_env(ets_probe, key, default)].

%% Calls that ERTS runs and PBeam does not support yet.
unsupported(Case) ->
    case Case of
        bag -> ets:new(ets_probe, [bag]);
        set_after_bag -> ets:new(ets_probe, [bag, set]);
        duplicate_bag -> ets:new(ets_probe, [duplicate_bag]);
        ordered_set -> ets:new(ets_probe, [ordered_set]);
        heir -> ets:new(ets_probe, [{heir, self()}]);
        heir_data -> ets:new(ets_probe, [{heir, self(), data}]);
        keypos -> ets:insert(ets:new(ets_probe, [{keypos, id(1 bsl 32)}]), {a});
        index ->
            T = ets:new(ets_probe, []),
            true = ets:insert(T, {k, v}),
            ets:lookup_element(T, k, 1 bsl 32, d)
    end.

start(Options) ->
    Parent = self(),
    Owner = spawn(?MODULE, own, [Parent, Options]),
    receive
        {Owner, Table} -> {Owner, Table}
    end.

own(Parent, ac_tab) ->
    own(Parent, ac_tab, [set, public, named_table, {read_concurrency, true}]);
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

%% Keep an integer outside 0..255 out of a literal: PBeam loads only
%% SMALL_INTEGER_EXT integers in literals.
id(X) -> X.
