-module(exit_owned_probe).
-export([tables/2, table_gcs/2]).
-export([table_exiter/4, watcher/4]).

%% The ETS tables of an exiting process on the pinned ERTS JIT
%% (erts_continue_exit_process, erl_process.c:14270-14833).
%%
%% Each case races a process a, which exits after it uses all but
%% 3997 - K reductions of a turn, with a process b, which a spawns in that
%% turn. For each tag, b sends whether a still has its name and how many
%% tables of a still have their names. The collector returns the events
%% that it gets in arrival order, as runs {Event, Count}. With one
%% scheduler, the runs show the turn in which the exit of a deletes each
%% table and sends each DOWN message.
%%
%% Opts are the spawn options of a and b. The ERTS reference run gives them
%% a large heap, so that no garbage collection occurs. PBeam has no process
%% heap and gets [].

%% Case {Sizes, K}: a owns one public named table for each size, with the
%% objects {1} to {Size}. A size {Size, Deleted} then deletes the keys 1 to
%% Deleted again, so that the table has fewer buckets. A size {atoms, Size}
%% has the atom keys 'key1é' to 'keySizeé' in place of the integers: the
%% atom table hash of each key reads the two UTF-8 bytes of é as one byte,
%% and its long keys fold the high bits of the hash.
tables(Cases, Opts) ->
    [{Case, table_race(Case, Opts, false)} || Case <- Cases].

%% ERTS only: the number of garbage collections of a and b in each case.
table_gcs(Cases, Opts) ->
    [{Case, gc_count(table_race(Case, Opts, true))} || Case <- Cases].

gc_count(Runs) ->
    lists:sum([Count || {gc, Count} <- Runs]).

table_race({Sizes, K}, Opts, Trace) ->
    Root = self(),
    {A, _} = spawn_opt(?MODULE, table_exiter, [Root, Sizes, K, Opts], [monitor | Opts]),
    trace(Trace, A, Root),
    collect(1, false, []).

trace(false, _, _) -> false;
trace(true, A, Root) ->
    erlang:trace(A, true, [garbage_collection, set_on_spawn, {tracer, Root}]).

%% a registers its name, makes its tables, and switches out. In its next
%% turn it spawns b and exits: spawn_opt/4 1, bump_reductions/1 1 + K,
%% return 1.
table_exiter(Root, Sizes, K, Opts) ->
    register(exiter, self()),
    Names = make_tables(Sizes, [t1, t2, t3, t4, t5, t6]),
    erlang:yield(),
    erlang:yield(),
    spawn_opt(?MODULE, watcher, [Root, exiter, Names, 6000], Opts),
    erlang:bump_reductions(K),
    ok.

make_tables([], _) -> [];
make_tables([{atoms, Size} | Sizes], [Name | Names]) ->
    make_table(Name, atoms, Size, 0, Sizes, Names);
make_tables([{Size, Deleted} | Sizes], [Name | Names]) ->
    make_table(Name, integers, Size, Deleted, Sizes, Names);
make_tables([Size | Sizes], [Name | Names]) ->
    make_table(Name, integers, Size, 0, Sizes, Names).

make_table(Name, Kind, Size, Deleted, Sizes, Names) ->
    Name = ets:new(Name, [public, named_table]),
    fill(Name, Kind, Size),
    delete(Name, Deleted),
    [Name | make_tables(Sizes, Names)].

fill(_, _, 0) -> ok;
fill(Name, Kind, N) ->
    true = ets:insert(Name, {key(Kind, N)}),
    fill(Name, Kind, N - 1).

key(integers, N) -> N;
key(atoms, N) -> list_to_atom("key" ++ integer_to_list(N) ++ [233]).

delete(_, 0) -> ok;
delete(Name, N) ->
    true = ets:delete(Name, N),
    delete(Name, N - 1).

watcher(Collector, _, _, 0) ->
    Collector ! done;
watcher(Collector, Name, Tables, Count) ->
    Collector ! {b, whereis(Name) =/= undefined, named(Tables)},
    watcher(Collector, Name, Tables, Count - 1).

named([]) -> 0;
named([Name | Names]) ->
    case ets:whereis(Name) of
        undefined -> named(Names);
        _ -> 1 + named(Names)
    end.

%% Collect until b is done and each monitor of a gave its DOWN message.
collect(0, true, Acc) ->
    lists:reverse(Acc, []);
collect(Downs, Done, Acc) ->
    receive
        {b, _, _} = B -> collect(Downs, Done, event(B, Acc));
        {'DOWN', _, process, _, Reason} -> collect(Downs - 1, Done, event({down, Reason}, Acc));
        done -> collect(Downs, true, Acc);
        {trace, _, gc_minor_start, _} -> collect(Downs, Done, event(gc, Acc));
        {trace, _, _, _} -> collect(Downs, Done, Acc)
    end.

event(Kind, [{Kind, Count} | Acc]) -> [{Kind, Count + 1} | Acc];
event(Kind, Acc) -> [{Kind, 1} | Acc].
