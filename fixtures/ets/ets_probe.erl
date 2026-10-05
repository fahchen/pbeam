-module(ets_probe).
-export([new/0, owner_exit/0, unsupported/1, own/2]).

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

%% Calls that ERTS runs and PBeam does not support yet.
unsupported(Case) ->
    case Case of
        bag -> ets:new(ets_probe, [bag]);
        set_after_bag -> ets:new(ets_probe, [bag, set]);
        duplicate_bag -> ets:new(ets_probe, [duplicate_bag]);
        ordered_set -> ets:new(ets_probe, [ordered_set]);
        heir -> ets:new(ets_probe, [{heir, self()}]);
        heir_data -> ets:new(ets_probe, [{heir, self(), data}])
    end.

start(Options) ->
    Parent = self(),
    Owner = spawn(?MODULE, own, [Parent, Options]),
    receive
        {Owner, Table} -> {Owner, Table}
    end.

own(Parent, Options) ->
    Name = case lists:member(named_table, Options) of
               true -> ets_probe_owned;
               false -> ets_probe
           end,
    own(Parent, Name, Options).

own(Parent, Name, Options) ->
    Table = ets:new(Name, Options),
    Parent ! {self(), Table},
    receive
        stop -> ok
    end.

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
