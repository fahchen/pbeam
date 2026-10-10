-module(bif_audit_probe).
-export([costs/1, gcs/1]).
-export([measure/3, idle/0, traced/3, id/1]).

%% The reductions of each BIF of the PBeam BIF table that has a body, for
%% a small and a large input, on the pinned ERTS JIT. Each cost is the
%% reductions of call/2 in a new process, without the reductions of
%% call(none, _). It includes the cost of the call instruction: 1 for a
%% light BIF, 0 for a heavy BIF, and 0 for a guard BIF in a bif
%% instruction.
%%
%% Opts are the spawn options of the measured processes. The ERTS reference
%% run gives them a large heap, so that no garbage collection occurs. PBeam
%% has no process heap and gets [].

-define(REDS, element(2, process_info(self(), reductions))).

%% {Bif, [Input]}. small is a small input, and large is a large input where
%% the input has a size.
cases() ->
    [{times, [small, large]}, {plus, [small, large]}, {minus, [small, large]},
     {append, [small, large]}, {exact_eq, [small, large]}, {exact_ne, [small, large]},
     {ge, [small, large]}, {atom_to_list, [small, large]}, {bump_reductions, [small]},
     {cancel_timer, [small]}, {demonitor1, [small]}, {demonitor2, [small]},
     {int_div, [small]}, {element, [small, large]}, {error1, [small, large]},
     {error3, [small]}, {exit1, [small]}, {exit2, [small]},
     {function_exported, [small]}, {get, [small, large]}, {group_leader0, [small]},
     {hd, [small, large]}, {integer_to_list, [small]}, {iolist_size, [small, large]},
     {iolist_to_iovec, [small, large]}, {is_function1, [small]}, {is_function2, [small]},
     {is_map_key, [small, large]}, {is_process_alive, [small]}, {length, [small, large]},
     {link, [small]}, {list_to_atom, [small, large]}, {load_nif, [small]},
     {make_fun, [small]}, {make_ref, [small]}, {map_get, [small, large]},
     {min, [small, large]}, {monitor2, [small]}, {monitor3, [small]},
     {monotonic_time, [small]}, {nif_error, [small]}, {node0, [small]}, {node1, [small]},
     {'not', [small]}, {posixtime_to_universaltime, [small]}, {process_flag, [small]},
     {process_info, [small]}, {put, [small, large]}, {raise, [small]}, {register, [small]},
     {int_rem, [small]}, {self, [small]}, {send2, [small, large]}, {send3, [small]},
     {spawn, [small, large]}, {spawn_link, [small]}, {spawn_opt, [small]},
     {start_timer, [small]}, {system_flag, [small]}, {system_info, [small]},
     {tl, [small, large]}, {tuple_size, [small]}, {universaltime_to_localtime, [small]},
     {unlink, [small]}, {whereis, [small]}, {group_leader2, [small]},
     {group_leader3, [small]}, {is_process_alive2, [small]}, {map_next, [small]},
     {perf_counter_unit, [small]}, {ets_delete, [small, large]}, {ets_insert, [small]},
     {ets_lookup, [small, large]}, {ets_lookup_element, [small, large]},
     {ets_member, [small, large]}, {ets_new, [small]}, {ets_whereis, [small]},
     {printable_range, [small]}, {keyfind, [small, large]}, {keymember, [small, large]},
     {member, [small, large]}, {reverse, [small, large]}, {maps_find, [small, large]},
     {maps_from_list, [small]}, {maps_keys, [small]}, {maps_merge, [small]},
     {maps_remove, [small, large]}, {dflag_unicode_io, [small]}, {getenv, [small, large]},
     {set_signal, [small]}, {system_time, [small]}, {pt_get0, [small]},
     {pt_get1, [small, large]}, {pt_get2, [small]}, {pt_put, [small]},
     {re_compile, [small, large]}, {characters_to_binary, [small]},
     {characters_to_list, [small]}].

id(X) -> X.

copies(0, _) -> [];
copies(1, Term) -> [Term];
copies(Count, Term) ->
    Half = Count div 2,
    copies(Half, Term) ++ copies(Count - Half, Term).

pairs(First, Last) when First > Last -> [];
pairs(Key, Key) -> [{Key, Key}];
pairs(First, Last) ->
    Middle = (First + Last) div 2,
    pairs(First, Middle) ++ pairs(Middle + 1, Last).

big_list() -> copies(10000, 7).

%% A process that waits.
idle() ->
    receive stop -> ok end.

%% The argument of a case, built before the measurement.
arg(times, small) -> {id(3), id(4)};
arg(times, large) -> {id(1 bsl 29), id(1 bsl 28)};
arg(plus, small) -> {id(3), id(4)};
arg(plus, large) -> {id(1 bsl 57), id(1 bsl 57)};
arg(minus, small) -> {id(3), id(4)};
arg(minus, large) -> {id(-(1 bsl 57)), id(1 bsl 57)};
arg(append, small) -> {[1], [2]};
arg(append, large) -> {big_list(), [2]};
arg(exact_eq, small) -> {id(a), id(a)};
arg(exact_eq, large) -> {big_list(), big_list()};
arg(exact_ne, small) -> {id(a), id(b)};
arg(exact_ne, large) -> {big_list(), big_list()};
arg(ge, small) -> {id(2), id(1)};
arg(ge, large) -> {big_list(), big_list()};
arg(atom_to_list, small) -> id(ab);
arg(atom_to_list, large) -> list_to_atom(copies(255, $a));
arg(list_to_atom, small) -> "ab";
arg(list_to_atom, large) -> copies(255, $b);
arg(cancel_timer, small) -> erlang:start_timer(1000000, self(), tick);
arg(demonitor1, small) -> monitor(process, spawn(?MODULE, idle, []));
arg(demonitor2, small) -> monitor(process, spawn(?MODULE, idle, []));
arg(element, small) -> {1, 2, 3};
arg(element, large) -> {big_list(), 2, 3};
arg(error1, small) -> id(x);
arg(error1, large) -> big_list();
arg(exit2, small) -> spawn(?MODULE, idle, []);
arg(get, small) -> put(k, v);
arg(get, large) -> put_keys(10000), put(k, v);
arg(hd, small) -> [1, 2];
arg(hd, large) -> big_list();
arg(tl, small) -> [1, 2];
arg(tl, large) -> big_list();
arg(iolist_size, small) -> [1, 2];
arg(iolist_size, large) -> big_list();
arg(iolist_to_iovec, small) -> [1, 2];
arg(iolist_to_iovec, large) -> big_list();
arg(is_map_key, small) -> maps:from_list([{1, 1}]);
arg(is_map_key, large) -> maps:from_list(pairs(1, 10000));
arg(map_get, small) -> maps:from_list([{1, 1}]);
arg(map_get, large) -> maps:from_list(pairs(1, 10000));
arg(maps_find, small) -> maps:from_list([{1, 1}]);
arg(maps_find, large) -> maps:from_list(pairs(1, 10000));
arg(maps_remove, small) -> maps:from_list([{1, 1}]);
arg(maps_remove, large) -> maps:from_list(pairs(1, 10000));
arg(is_process_alive, small) -> spawn(?MODULE, idle, []);
arg(length, small) -> [1, 2];
arg(length, large) -> big_list();
arg(link, small) -> spawn(?MODULE, idle, []);
arg(unlink, small) -> Pid = spawn(?MODULE, idle, []), link(Pid), Pid;
arg(min, small) -> {id(1), id(2)};
arg(min, large) -> {big_list(), big_list()};
arg(monitor2, small) -> spawn(?MODULE, idle, []);
arg(monitor3, small) -> spawn(?MODULE, idle, []);
arg(put, small) -> none;
arg(put, large) -> put_keys(10000);
arg(send2, small) -> spawn(?MODULE, idle, []);
arg(send2, large) -> {spawn(?MODULE, idle, []), big_list()};
arg(send3, small) -> spawn(?MODULE, idle, []);
arg(spawn, small) -> [];
arg(spawn, large) -> [big_list()];
arg(group_leader2, small) -> spawn(?MODULE, idle, []);
arg(group_leader3, small) -> spawn(?MODULE, idle, []);
arg(is_process_alive2, small) -> spawn(?MODULE, idle, []);
arg(map_next, small) -> maps:from_list([{1, 1}]);
arg(ets_delete, small) -> table(small);
arg(ets_delete, large) -> table(large);
arg(ets_insert, small) -> ets:new(audit, [public]);
arg(ets_lookup, Input) -> table(Input);
arg(ets_lookup_element, Input) -> table(Input);
arg(ets_member, Input) -> table(Input);
arg(ets_whereis, small) -> ets:new(audit_named, [named_table, public]);
arg(keyfind, small) -> [{1, a}];
arg(keyfind, large) -> pairs(1, 10000);
arg(keymember, small) -> [{1, a}];
arg(keymember, large) -> pairs(1, 10000);
arg(member, small) -> [1, 2];
arg(member, large) -> big_list();
arg(reverse, small) -> [1, 2];
arg(reverse, large) -> big_list();
arg(getenv, small) -> "PBEAM_AUDIT_MISSING";
arg(getenv, large) -> copies(1000, $X);
arg(pt_get1, small) -> persistent_term:put({audit, small}, 1), {audit, small};
arg(pt_get1, large) -> persistent_term:put({audit, large}, big_list()), {audit, large};
arg(re_compile, small) -> "a";
arg(re_compile, large) -> copies(1000, $a);
arg(_, _) -> none.

%% Put the keys Count..1, each with itself as its value. The loop builds
%% no list: PBeam copies a list at each cons, so a list of the keys took
%% O(N^2) time.
put_keys(0) -> ok;
put_keys(Count) ->
    put(Count, Count),
    put_keys(Count - 1).

%% A public set table with the key 1, and for large a value of 10000
%% elements.
table(small) ->
    Table = ets:new(audit, [public]),
    ets:insert(Table, {1, a}),
    Table;
table(large) ->
    Table = ets:new(audit, [public]),
    ets:insert(Table, {1, big_list()}),
    Table.

%% Call a BIF with the argument of a case. The list keeps the result, so
%% the call is a body call.
call(times, {A, B}) -> [A * B];
call(plus, {A, B}) -> [A + B];
call(minus, {A, B}) -> [A - B];
call(append, {A, B}) -> [A ++ B];
call(exact_eq, {A, B}) -> [erlang:'=:='(A, B)];
call(exact_ne, {A, B}) -> [erlang:'=/='(A, B)];
call(ge, {A, B}) -> [erlang:'>='(A, B)];
call(atom_to_list, Atom) -> [atom_to_list(Atom)];
call(bump_reductions, _) -> [erlang:bump_reductions(10)];
call(cancel_timer, Ref) -> [erlang:cancel_timer(Ref, [])];
call(demonitor1, Ref) -> [demonitor(Ref)];
call(demonitor2, Ref) -> [demonitor(Ref, [flush])];
call(int_div, _) -> [id(7) div id(2)];
call(element, Tuple) -> [element(2, Tuple)];
call(error1, Reason) ->
    [try erlang:error(Reason)
     catch error:_ -> caught
     end];
call(error3, _) ->
    [try erlang:error(x, [a], [])
     catch error:_ -> caught
     end];
call(exit1, _) ->
    [try exit(x)
     catch exit:_ -> caught
     end];
call(exit2, Pid) -> [exit(Pid, normal)];
call(function_exported, _) -> [erlang:function_exported(?MODULE, costs, 1)];
call(get, _) -> [get(k)];
call(group_leader0, _) -> [group_leader()];
call(hd, List) -> [hd(List)];
call(integer_to_list, _) -> [integer_to_list(id(12345))];
call(iolist_size, List) -> [iolist_size(List)];
call(iolist_to_iovec, List) -> [erlang:iolist_to_iovec(List)];
call(is_function1, _) -> [is_function(id(x))];
call(is_function2, _) -> [is_function(id(x), 1)];
call(is_map_key, Map) -> [is_map_key(1, Map)];
call(is_process_alive, Pid) -> [is_process_alive(Pid)];
call(length, List) -> [length(List)];
call(link, Pid) -> [link(Pid)];
call(list_to_atom, List) -> [list_to_atom(List)];
call(load_nif, _) -> [apply(id(erlang), id(load_nif), ["none", 0])];
call(make_fun, _) -> [erlang:make_fun(?MODULE, id, 1)];
call(make_ref, _) -> [make_ref()];
call(map_get, Map) -> [map_get(1, Map)];
call(min, {A, B}) -> [min(A, B)];
call(monitor2, Pid) -> [monitor(process, Pid)];
call(monitor3, Pid) -> [monitor(process, Pid, [])];
call(monotonic_time, _) -> [erlang:monotonic_time(millisecond)];
call(nif_error, _) ->
    [try erlang:nif_error(x)
     catch error:_ -> caught
     end];
call(node0, _) -> [node()];
call(node1, _) -> [node(self())];
call('not', _) -> [erlang:'not'(id(true))];
call(posixtime_to_universaltime, _) -> [erlang:posixtime_to_universaltime(0)];
call(process_flag, _) -> [process_flag(trap_exit, false)];
call(process_info, _) -> [process_info(self(), message_queue_len)];
call(put, _) -> [put(new_key, v)];
call(raise, _) ->
    [try erlang:raise(error, x, [])
     catch error:_ -> caught
     end];
call(register, _) -> [register(audit_name, self())];
call(int_rem, _) -> [id(7) rem id(2)];
call(self, _) -> [self()];
call(send2, {Pid, Message}) -> [erlang:send(Pid, Message)];
call(send2, Pid) -> [erlang:send(Pid, x)];
call(send3, Pid) -> [erlang:send(Pid, x, [])];
call(spawn, Args) -> [spawn(?MODULE, id, [Args])];
call(spawn_link, _) -> [spawn_link(?MODULE, id, [x])];
call(spawn_opt, _) -> [spawn_opt(?MODULE, id, [x], [])];
call(start_timer, _) -> [erlang:start_timer(1000000, self(), x)];
call(system_flag, _) -> [erlang:system_flag(system_logger, logger)];
call(system_info, _) -> [erlang:system_info(os_type)];
call(tl, List) -> [tl(List)];
call(tuple_size, _) -> [tuple_size(id({1, 2}))];
call(universaltime_to_localtime, _) ->
    [erlang:universaltime_to_localtime({{2020, 1, 1}, {0, 0, 0}})];
call(unlink, Pid) -> [unlink(Pid)];
call(whereis, _) -> [whereis(audit_missing)];
call(group_leader2, Pid) -> [erts_internal:group_leader(group_leader(), Pid)];
call(group_leader3, Pid) -> [erts_internal:group_leader(group_leader(), Pid, make_ref())];
call(is_process_alive2, Pid) -> [erts_internal:is_process_alive(Pid, make_ref())];
call(map_next, Map) -> [erts_internal:map_next(0, Map, [])];
call(perf_counter_unit, _) -> [erts_internal:perf_counter_unit()];
call(ets_delete, Table) -> [ets:delete(Table, 1)];
call(ets_insert, Table) -> [ets:insert(Table, {1, a})];
call(ets_lookup, Table) -> [ets:lookup(Table, 1)];
call(ets_lookup_element, Table) -> [ets:lookup_element(Table, 1, 2, none)];
call(ets_member, Table) -> [ets:member(Table, 1)];
call(ets_new, _) -> [ets:new(audit, [])];
call(ets_whereis, _) -> [ets:whereis(audit_named)];
call(printable_range, _) -> [io:printable_range()];
call(keyfind, List) -> [lists:keyfind(0, 1, List)];
call(keymember, List) -> [lists:keymember(0, 1, List)];
call(member, List) -> [lists:member(0, List)];
call(reverse, List) -> [lists:reverse(List, [])];
call(maps_find, Map) -> [maps:find(1, Map)];
call(maps_from_list, _) -> [maps:from_list([{1, 1}])];
call(maps_keys, _) -> [maps:keys(id(#{a => 1}))];
call(maps_merge, _) -> [maps:merge(id(#{a => 1}), id(#{b => 2}))];
call(maps_remove, Map) -> [maps:remove(1, Map)];
call(dflag_unicode_io, _) -> [net_kernel:dflag_unicode_io(self())];
call(getenv, Name) -> [os:getenv(Name)];
call(set_signal, _) -> [os:set_signal(sigusr2, default)];
call(system_time, _) -> [os:system_time(millisecond)];
call(pt_get0, _) -> [persistent_term:get()];
call(pt_get1, Key) -> [persistent_term:get(Key)];
call(pt_get2, _) -> [persistent_term:get(audit_missing, none)];
call(pt_put, _) -> [persistent_term:put(audit_put, make_ref())];
call(re_compile, Pattern) -> [re:compile(Pattern, [])];
call(characters_to_binary, _) -> [unicode:characters_to_binary("ab", unicode)];
call(characters_to_list, _) -> [unicode:characters_to_list(<<"ab">>, unicode)];
call(none, _) -> [].

%% For each BIF and input, the reductions of call/2 in a new process,
%% without the reductions of call(none, _).
costs(Opts) ->
    Base = delta(none, small, Opts),
    [{Bif, [delta(Bif, Input, Opts) - Base || Input <- Inputs]} || {Bif, Inputs} <- cases()].

delta(Bif, Input, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), Bif, Input], Opts),
    receive {Pid, Delta} -> Delta end.

%% Build the argument, and switch out, so that the BIF runs with the
%% budget of a new turn.
measure(Collector, Bif, Input) ->
    Arg = arg(Bif, Input),
    erlang:yield(),
    Before = ?REDS,
    call(Bif, Arg),
    Collector ! {self(), ?REDS - Before}.

%% ERTS only: the number of garbage collections of each measured process.
gcs(Opts) ->
    [{Bif, Input, gc_count(Bif, Input, Opts)}
     || {Bif, Inputs} <- [{none, [small]} | cases()], Input <- Inputs].

gc_count(Bif, Input, Opts) ->
    Self = self(),
    {Pid, Ref} = spawn_opt(?MODULE, traced, [Self, Bif, Input], [monitor | Opts]),
    erlang:trace(Pid, true, [garbage_collection, {tracer, Self}]),
    Pid ! go,
    gc_events(Ref, 0).

traced(Collector, Bif, Input) ->
    receive go -> ok end,
    measure(Collector, Bif, Input).

gc_events(Ref, Count) ->
    receive
        {'DOWN', Ref, _, _, _} -> Count;
        {trace, _, gc_minor_start, _} -> gc_events(Ref, Count + 1);
        {trace, _, gc_major_start, _} -> gc_events(Ref, Count + 1);
        _ -> gc_events(Ref, Count)
    end.
