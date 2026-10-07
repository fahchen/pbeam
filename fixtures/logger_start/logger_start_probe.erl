-module(logger_start_probe).
-export([reverse/0, remove/0, without/0, ets/0, ets_access/0, find/0, from_list/0, is_key/0,
         to_list/0, map_next/0, next/0, ordered/0, ordered_second_call_badarg/0,
         demonitor/0, unlink/0, hd/0, system_logger/0,
         message_queue_data/0, system_flag/0, process_flag/0, unsupported_flag/1, raise/0,
         raise_throw/0, map_paths/0]).

%% `lists:reverse/2` (erl_bif_lists.c:1246-1263). logger_simple_h:replay_buffer/1
%% (logger_simple_h.erl:150) calls it with the buffer and the drop message, and
%% lists:reverse/1 (lists.erl:298-299) calls it for three or more elements. An
%% empty first list returns the second term, and a first term that is not a
%% list raises badarg.
reverse() ->
    [lists:reverse(id([a, b, c]), id([d])),
     lists:reverse(id([]), id(foo)),
     lists:reverse(id([a]), id([])),
     lists:reverse(id([]), id([])),
     lists:reverse(id([a, b, c, d])),
     try lists:reverse(id(foo), id([])) catch error:R1 -> R1 end,
     try lists:reverse(id(<<1>>), id(foo)) catch error:R2 -> R2 end].

%% `maps:remove/2` (erl_map.c:1929-1937). A key that the map does not hold
%% returns the map. A hashmap that keeps 32 keys becomes a flatmap
%% (erl_map.c:2989-3025), and a term that is not a map raises {badmap, Map}.
%% The keys are integers: the ERTS index of a new atom can change from one
%% run to the next, and with it the flatmap order.
remove() ->
    M = id(#{1 => one, 2 => two}),
    [maps:remove(1, M),
     maps:remove(3, M),
     maps:remove(id({x}), M),
     maps:remove(one, id(#{one => 1})),
     maps:keys(maps:remove(33, id(big(33)))),
     maps:keys(maps:remove(40, id(big(33)))),
     maps:keys(maps:remove(id({x}), id(big(33)))),
     maps:keys(maps:remove(1, id(big(34)))),
     maps:keys(drop(1, 30, id(big(64)))),
     try maps:remove(a, id(foo)) catch error:R1 -> R1 end,
     try maps:remove(a, id([])) catch error:R2 -> R2 end].

%% maps:without/2 (maps.erl:1139-1140) folds `fun maps:remove/2` over the
%% keys. logger_h_common:adding_handler/1 (logger_h_common.erl:71) removes
%% keys that the handler config does not hold, and
%% logger_h_common:filter_config/1 (logger_h_common.erl:187-188) removes
%% the olp key that it holds.
without() ->
    [maps:without(id([sync_mode_qlen, filesync_repeat_interval]), id(#{type => standard_io})),
     maps:without(id([olp]), id(#{olp => {logger_olp, x}, 1 => one, 2 => two}))].

%% `ets:whereis/1` (erl_db.c:2849-2872), `ets:lookup/2` (erl_db.c:2877-2900),
%% `ets:member/2` (erl_db.c:2905-2928), and `ets:delete/2`
%% (erl_db.c:3328-3349). logger_config:exist/1 (logger_config.erl:45-49) finds
%% the logger table with whereis/1, logger_config:get/2 and exist/2
%% (logger_config.erl:87-98) read it, and logger_config:delete/2
%% (logger_config.erl:51-53) removes a handler key. A set table compares keys
%% with exact equality. A name that is not an atom, a free name, a term that is
%% not an atom or a reference, and a missing access right raise badarg.
ets() ->
    T = ets:new(logger_start_probe, [set, protected, named_table, {read_concurrency, true}, {write_concurrency, true}]),
    Tid = ets:whereis(logger_start_probe),
    true = ets:insert(Tid, [{a, 1}, {{h, b}, 2}, {[x], 3}]),
    U = ets:new(other, [public, {keypos, 2}]),
    true = ets:insert(U, {v, k}),
    R = [T, Tid =/= T, Tid =:= ets:whereis(id(logger_start_probe)), ets:whereis(id(other)), ets:whereis(id(missing)),
         ets:lookup(T, a), ets:lookup(Tid, {h, b}), ets:lookup(Tid, [x]), ets:lookup(T, b), ets:lookup(U, k), ets:lookup(U, v),
         ets:member(T, a), ets:member(Tid, {h, b}), ets:member(T, b), ets:member(U, k),
         ets:delete(T, a), ets:delete(Tid, missing), ets:delete(U, k), ets:lookup(T, a), ets:member(U, k), ets:lookup(Tid, {h, b}),
         catch_error(fun() -> ets:whereis(id("logger")) end),
         catch_error(fun() -> ets:whereis(id(1)) end),
         catch_error(fun() -> ets:lookup(id(missing), a) end),
         catch_error(fun() -> ets:lookup(id(1), a) end),
         catch_error(fun() -> ets:member(id(missing), a) end),
         catch_error(fun() -> ets:member(id({}), a) end),
         catch_error(fun() -> ets:delete(id(missing), a) end),
         catch_error(fun() -> ets:delete(id([]), a) end)],
    R.

%% The access rights of another process. `ets:whereis/1` needs only DB_INFO
%% (erl_db_util.h:451), so it finds a private table of another process.
%% lookup/2 and member/2 need DB_READ, and delete/2 needs DB_WRITE
%% (erl_db_util.h:449-450). The exit of the owner deletes its table and frees
%% its name.
ets_access() ->
    _ = ets:new(lsp_protected, [named_table]),
    _ = ets:new(lsp_private, [named_table, private]),
    true = ets:insert(lsp_protected, {k, v}),
    true = ets:insert(lsp_private, {k, v}),
    Self = self(),
    {Pid, Ref} = spawn_monitor(fun() ->
        Self ! {self(), [ets:lookup(lsp_protected, k), ets:member(lsp_protected, k),
                         catch_error(fun() -> ets:delete(lsp_protected, k) end),
                         catch_error(fun() -> ets:lookup(lsp_private, k) end),
                         catch_error(fun() -> ets:member(lsp_private, k) end),
                         catch_error(fun() -> ets:delete(lsp_private, k) end),
                         ets:whereis(lsp_private) =/= undefined]},
        _ = ets:new(lsp_gone, [named_table, public])
    end),
    receive {Pid, Result} -> ok end,
    receive {'DOWN', Ref, process, Pid, normal} -> ok end,
    Result ++ [ets:whereis(lsp_gone), catch_error(fun() -> ets:lookup(lsp_gone, k) end)].

%% `maps:find/2` (erl_map.c:225-243). supervisor:find_child/2
%% (supervisor.erl:1871-1872) finds a child with it. A key that the map does
%% not hold gives error, and a term that is not a map raises {badmap, Map}.
find() ->
    M = id(#{a => 1, [b] => 2, 3 => three}),
    [maps:find(a, M), maps:find([b], M), maps:find(3, M), maps:find(c, M), maps:find(a, id(#{})),
     maps:find(40, id(big(40))), maps:find(41, id(big(40))),
     catch_error(fun() -> maps:find(a, id(foo)) end),
     catch_error(fun() -> maps:find(a, id([])) end)].

%% `maps:from_list/1` (erl_map.c:321-363). maps:with/2 (maps.erl:1165-1166)
%% calls it, from logger_h_common:adding_handler/1
%% (logger_h_common.erl:74-82) and logger_std_h (logger_std_h.erl:272). A
%% later pair of a key wins, and 43 pairs with 3 keys give a flatmap. A term
%% that is not a proper list, an element that is not a tuple, and a tuple
%% with an arity other than 2 raise badarg.
from_list() ->
    [maps:from_list(id([])),
     maps:from_list(id([{2, b}, {1, a}, {2, c}])),
     maps:from_list(id([{type, standard_io}])),
     maps:keys(maps:from_list(id([{N, N} || N <- lists:seq(1, 40)]))),
     maps:from_list(id([{N - 1, N} || N <- lists:seq(1, 40), N < 4] ++ [{2, N} || N <- lists:seq(41, 80)])),
     catch_error(fun() -> maps:from_list(id(foo)) end),
     catch_error(fun() -> maps:from_list(id([a])) end),
     catch_error(fun() -> maps:from_list(id([{a, 1, 2}])) end),
     catch_error(fun() -> maps:from_list(id([{a, 1} | b])) end)].

%% `erlang:is_map_key/2` (erl_map.c:1185-1196). logger:add_handler/3
%% (logger.erl:832) runs it in a guard. A term that is not a map raises
%% {badmap, Map}, and in a guard the bif instruction goes to its fail label.
is_key() ->
    M = id(#{a => 1}),
    [is_map_key(a, M), is_map_key(b, M), is_map_key(40, id(big(40))), is_map_key(41, id(big(40))),
     key_guard(a, M), key_guard(b, M), key_guard(a, id(foo)),
     catch_error(fun() -> is_map_key(a, id(foo)) end),
     catch_error(fun() -> erlang:is_map_key(a, id([])) end)].

key_guard(K, M) when is_map_key(K, M) -> yes;
key_guard(_, _) -> no.

%% `erts_internal:map_next/3` (erl_map.c:3593-4135), through maps:to_list/1
%% (maps.erl:525-542). logger_h_common:filter_config/1
%% (logger_h_common.erl:441), logger_std_h (logger_std_h.erl:306),
%% logger_olp (logger_olp.erl:418), and logger_formatter
%% (logger_formatter.erl:721) call it. A flatmap gives all pairs in one call.
%% A hashmap gives the pairs in the HAMT order, in chunks.
to_list() ->
    [maps:to_list(id(#{})),
     maps:to_list(id(#{2 => b, 1 => a, three => 3})),
     maps:to_list(id(#{type => standard_io})),
     maps:to_list(id(big(40))),
     maps:to_list(id(big(100)))].

%% Direct `erts_internal:map_next/3` calls in list mode and iterator mode. The
%% calls run in a new process, which has its full reduction budget, so a
%% hashmap call at path 0 gives 32 pairs (MAP_SMALL_MAP_LIMIT) and the next
%% call gives the rest. A flatmap needs a small path from 0 to its size. A
%% term that is not a map, a third argument that is not iterator or a list,
%% and a path that selects no entry of a node raise badarg.
map_next() ->
    in_new_process(fun() ->
        Big = id(big(40)),
        [Path, Big | First] = erts_internal:map_next(0, Big, []),
        Rest = erts_internal:map_next(Path, Big, First),
        [Path2, Big | First2] = erts_internal:map_next(0, Big, [end_]),
        [erts_internal:map_next(0, id(#{2 => b, 1 => a}), []),
         erts_internal:map_next(0, id(#{2 => b, 1 => a}), [x]),
         erts_internal:map_next(2, id(#{2 => b, 1 => a}), []),
         erts_internal:map_next(0, id(#{2 => b, 1 => a}), iterator),
         erts_internal:map_next(0, id(#{}), iterator),
         Path, length(First), First, Rest,
         Path2, lists:last(First2),
         catch_error(fun() -> erts_internal:map_next(3, id(#{2 => b, 1 => a}), []) end),
         catch_error(fun() -> erts_internal:map_next(-1, id(#{a => 1}), []) end),
         catch_error(fun() -> erts_internal:map_next(a, id(#{a => 1}), []) end),
         catch_error(fun() -> erts_internal:map_next(0, id(foo), []) end),
         catch_error(fun() -> erts_internal:map_next(0, id(#{a => 1}), foo) end),
         catch_error(fun() -> erts_internal:map_next(a, Big, []) end),
         catch_error(fun() -> erts_internal:map_next(16#ff, Big, []) end),
         catch_error(fun() -> erts_internal:map_next(-1, Big, iterator) end)]
    end).

%% `erts_internal:map_next/3` paths that reach a collision node and paths
%% that are bignums. Lists A and B have equal map hashes, so they share a
%% collision node at depth 16. A chunk takes the whole collision node, so path
%% 0 gives 33 pairs. Path 113 selects the collision node. A path other than 0
%% has no limit of 32 pairs: 2^59 (a bignum of one digit) and -(2^59) walk
%% from the first leaf to the end, and -(2^60) (a negative bignum) raises
%% badarg. A flatmap needs a small path. The Deep keys share their low 60
%% hash bits in two groups, so a path of 16 elements, such as 2^60 + 18 and
%% 2^60 + 786, selects a node at level 15.
map_paths() ->
    in_new_process(fun() ->
        A = [246, 157, 203, 77, 19, 254, 19, 93],
        B = [161, 92, 65, 50, 231, 24, 153, 137],
        M1 = identity([19, 213, 340, 110, 91, 164, 399, 139, 361, 212, 41, 114, 100, 29, 347,
                       365, 171, 140, 13, 42, 149, 55, 2, 146, 372, 317, 191, 316, 202, 192,
                       233, B, A, 312, 98, 333]),
        M2 = identity([202, 192, 233, B, A, 312, 98, 333, 7, 398, 285, 90, 343, 38, 325, 326,
                       284, 281, 56, 153, 61, 99, 113, 383, 201, 237, 314, 65, 322, 137, 141,
                       308, 315, 230, 104, 187]),
        %% Each Deep key is an operand, not a literal list: PBeam reads no
        %% bignum in the literal table.
        Deep = [id(329898321527737274), id(446694106686689078), id(413866500823436792),
                id(-101714049538469044), id(-74925715755171563)],
        M3 = identity([319, 169, 371, 359, 160, 388, 136, 324, 348, 287, 296, 357, 239, 179,
                       289, 263, 313, 235, 226, 240, 211, 128, 247, 194, 384, 269, 168, 373,
                       265 | Deep]),
        M4 = identity(Deep ++ [126, 390, 155, 165, 228, 227, 242, 151, 208, 197, 350, 133, 301,
                               392, 196, 205, 332, 353, 177, 225, 142, 223, 275, 200, 377, 145,
                               107, 108, 184, 255, 272]),
        [iterator_keys(erts_internal:map_next(0, M1, iterator)),
         iterator_keys(erts_internal:map_next(113, M1, iterator)),
         iterator_keys(erts_internal:map_next(id(1 bsl 59), M1, iterator)),
         iterator_keys(erts_internal:map_next(id(-(1 bsl 59)), M1, iterator)),
         catch_error(fun() -> erts_internal:map_next(id(-(1 bsl 60)), M1, iterator) end),
         list_keys(erts_internal:map_next(0, M2, [])),
         catch_error(fun() -> erts_internal:map_next(id(1 bsl 59), id(#{1 => 1}), []) end),
         iterator_keys(erts_internal:map_next(id((1 bsl 60) + 18), M3, iterator)),
         list_keys(erts_internal:map_next(id((1 bsl 60) + 786), M4, []))]
    end).

identity(Keys) -> maps:from_list([{K, K} || K <- Keys]).

%% The keys of an iterator-mode result and the path that it ends with.
iterator_keys(none) -> {[], none};
iterator_keys([Path | _]) -> {[], Path};
iterator_keys({K, _, Next}) ->
    {Keys, Path} = iterator_keys(Next),
    {[K | Keys], Path}.

%% The keys of a list-mode result and the path that it starts with.
list_keys([Path, _ | Pairs]) when is_integer(Path) -> {[K || {K, _} <- Pairs], Path};
list_keys(Pairs) -> {[K || {K, _} <- Pairs], none}.

%% maps:next/1 (maps.erl:1094-1115) on maps:iterator/1 (maps.erl:1000, 1060),
%% which is [0 | Map]. The iterator of a hashmap holds 32 pairs and then
%% [Path | Map].
next() ->
    in_new_process(fun() ->
        Big = id(big(40)),
        {K1, V1, I1} = maps:next(maps:iterator(Big)),
        [keys_of(maps:iterator(id(#{2 => b, 1 => a}))),
         keys_of(maps:iterator(Big)),
         {K1, V1},
         element(3, erts_internal:map_next(0, Big, iterator)) =:= I1,
         iterator_path(erts_internal:map_next(0, Big, iterator)),
         erts_internal:map_next(iterator_path(erts_internal:map_next(0, Big, iterator)), Big, iterator) =/= none]
    end).

%% The ordered iterator: a path that is a list of keys (maps.erl:1062-1072).
%% The BIF gives the keys in the order of the list, in chunks of at most
%% CONTEXT_REDS / 32 keys (erl_map.c:3654-3660), so 130 keys need two calls.
%% A key that the map does not hold, a tail that is not [], and a list path
%% in list mode raise badarg. The second-call badarg case is in
%% ordered_second_call_badarg/0, because ERTS crashes on it.
ordered() ->
    in_new_process(fun() ->
        M = id(#{b => 2, a => 1, c => 3}),
        [keys_of([[a, b, c] | M]),
         keys_of([lists:reverse(lists:seq(1, 40)) | id(big(40))]),
         keys_of([lists:seq(1, 130) | id(big(130))]) =:= lists:seq(1, 130),
         erts_internal:map_next([c, a], M, iterator),
         erts_internal:map_next([], M, iterator),
         catch_error(fun() -> erts_internal:map_next([d], M, iterator) end),
         catch_error(fun() -> erts_internal:map_next([a | b], M, iterator) end),
         catch_error(fun() -> erts_internal:map_next([a], M, []) end)]
    end).

%% The ordered iterator raises badarg when the tail of the key list is not []
%% (here b), also when the badarg is in the second call, after the first chunk
%% of keys. On ERTS 29.1.1 (JIT) this crashes the emulator with SIGSEGV in
%% about one run in four: the BIF builds a tuple per key in a heap fragment
%% and raises badarg before it sets the last tuple's third element
%% (erl_map.c:3670-3707). HRelease keeps the half-built tuples in the
%% fragment, the GC after the BIF call copies and scans the fragment
%% (collect_live_heap_frags, erl_gc.c), and the unset element, which holds
%% uninitialised heap memory, reads as a wild boxed or list pointer. Because
%% the reference run would crash, the Rust test checks only PBeam here and
%% records the ERTS crash in docs/erts-differences.md.
ordered_second_call_badarg() ->
    in_new_process(fun() ->
        catch_error(fun() -> keys_of([lists:seq(1, 130) ++ b | id(big(130))]) end)
    end).

keys_of(I) ->
    case maps:next(I) of
        none -> [];
        {K, _, Next} -> [K | keys_of(Next)]
    end.

iterator_path({_, _, Next}) -> iterator_path(Next);
iterator_path([Path | _]) -> Path.

%% `erlang:demonitor/1` (bif.c:598-611). logger_simple_h:adding_handler/1
%% (logger_simple_h.erl:45) removes its monitor of the new handler process.
%% It runs the demonitor/2 step with [] and gives true, also when no active
%% monitor was removed. A DOWN message in the queue stays, and a term that is
%% not a reference raises badarg.
demonitor() ->
    {Pid, Ref} = spawn_monitor(fun() -> receive stop -> ok end end),
    R1 = erlang:demonitor(Ref),
    Pid ! stop,
    Ref2 = erlang:monitor(process, Pid),
    receive {'DOWN', Ref2, process, Pid, _} -> ok end,
    {Pid3, Ref3} = spawn_monitor(fun() -> ok end),
    Ref4 = erlang:monitor(process, Pid3),
    receive {'DOWN', Ref4, process, Pid3, _} -> ok end,
    R3 = erlang:demonitor(Ref3),
    Queued = receive {'DOWN', Ref3, process, Pid3, Reason} -> Reason after 0 -> none end,
    [R1, receive {'DOWN', Ref, _, _, _} -> down after 0 -> none end,
     R3, Queued, erlang:demonitor(Ref3), erlang:demonitor(make_ref()),
     catch_error(fun() -> erlang:demonitor(id(foo)) end),
     catch_error(fun() -> erlang:demonitor(id(self())) end)].

%% `erlang:unlink/1` (bif.c:1223-1364). logger_simple_h:loop/1
%% (logger_simple_h.erl:124) unlinks from the logger process. A local pid
%% gives true, also for a pid without a link, a process that is gone, and the
%% caller. An EXIT message in the queue stays. Each other term raises badarg.
unlink() ->
    in_new_process(fun() ->
        Child = spawn_link(fun() -> receive stop -> exit(boom) end end),
        R1 = unlink(Child),
        Ref = erlang:monitor(process, Child),
        Child ! stop,
        receive {'DOWN', Ref, process, Child, boom} -> ok end,
        process_flag(trap_exit, true),
        Child2 = spawn_link(fun() -> exit(bang) end),
        Ref2 = erlang:monitor(process, Child2),
        receive {'DOWN', Ref2, process, Child2, bang} -> ok end,
        R2 = unlink(Child2),
        Exit = receive {'EXIT', Child2, Why} -> Why after 0 -> none end,
        [R1, R2, Exit, unlink(self()), unlink(Child),
         catch_error(fun() -> unlink(id(foo)) end),
         catch_error(fun() -> unlink(id(make_ref())) end)]
    end).

%% `erlang:hd/1` (bif.c:2918-2925). gen_server:decode_msg/6
%% (gen_server.erl:2380) runs it in a guard. It gives the head of a proper
%% or an improper list. Each other term raises badarg, and in a guard the bif
%% instruction goes to its fail label.
hd() ->
    [hd(id([a, b])), hd(id([a | b])), erlang:hd(id([c])),
     hd_guard(id([a])), hd_guard(id([])), hd_guard(id(foo)),
     catch_error(fun() -> hd(id([])) end),
     catch_error(fun() -> hd(id(foo)) end)].

hd_guard(L) when hd(L) =:= a -> yes;
hd_guard(_) -> no.

%% `erlang:system_flag(system_logger, Logger)` (bif.c:5294-5296,
%% erl_trace.c:2486-2491). logger_proxy:init/1 (logger_proxy.erl:117) sets
%% itself as the system logger. `logger`, `undefined`, and a local pid set the
%% node value and give the old value. The start value is `logger`
%% (erl_trace.c:339). Each other value raises badarg.
system_logger() ->
    Old = erlang:system_flag(system_logger, logger),
    Self = self(),
    R = [erlang:system_flag(system_logger, undefined),
         erlang:system_flag(system_logger, Self),
         erlang:system_flag(system_logger, logger) =:= Self,
         erlang:system_flag(system_logger, logger),
         catch_error(fun() -> erlang:system_flag(system_logger, id(foo)) end),
         catch_error(fun() -> erlang:system_flag(system_logger, id(1)) end),
         catch_error(fun() -> erlang:system_flag(system_logger, id({logger})) end)],
    _ = erlang:system_flag(system_logger, Old),
    R.

%% `erlang:system_flag/2` (bif.c:4972-5304). An item that ERTS does not know
%% raises badarg, also `time_offset` with a value that is not `finalize`.
%% `debug_flags` gives true for each value. An item that needs a subsystem
%% that PBeam does not have also raises badarg for a value that ERTS rejects
%% before it uses the subsystem.
system_flag() ->
    [catch_error(fun() -> erlang:system_flag(id(no_such_flag), 1) end),
     catch_error(fun() -> erlang:system_flag(id(1), 1) end),
     catch_error(fun() -> erlang:system_flag(id(time_offset), id(preliminary)) end),
     erlang:system_flag(debug_flags, id(foo)),
     catch_error(fun() -> erlang:system_flag(multi_scheduling, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(schedulers_online, id(0)) end),
     catch_error(fun() -> erlang:system_flag(schedulers_online, id(a)) end),
     catch_error(fun() -> erlang:system_flag(dirty_cpu_schedulers_online, id(0)) end),
     catch_error(fun() -> erlang:system_flag(fullsweep_after, id(-1)) end),
     catch_error(fun() -> erlang:system_flag(fullsweep_after, id(1 bsl 59)) end),
     catch_error(fun() -> erlang:system_flag(min_heap_size, id(a)) end),
     catch_error(fun() -> erlang:system_flag(min_bin_vheap_size, id(-1)) end),
     catch_error(fun() -> erlang:system_flag(backtrace_depth, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(trace_control_word, id(-1)) end),
     catch_error(fun() -> erlang:system_flag(trace_control_word, id(1 bsl 32)) end),
     catch_error(fun() -> erlang:system_flag(trace_control_word, id(a)) end),
     catch_error(fun() -> erlang:system_flag(sequential_tracer, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(sequential_tracer, id([])) end),
     catch_error(fun() -> erlang:system_flag(sequential_tracer, id({1, bar})) end),
     catch_error(fun() -> erlang:system_flag(scheduler_wall_time, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(microstate_accounting, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(outstanding_system_requests_limit, id(0)) end),
     catch_error(fun() -> erlang:system_flag(outstanding_system_requests_limit, id(1 bsl 27)) end),
     catch_error(fun() -> erlang:system_flag(outstanding_system_requests_limit, id(a)) end),
     catch_error(fun() -> erlang:system_flag(scheduling_statistics, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(internal_cpu_topology, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(internal_cpu_topology, id([])) end),
     catch_error(fun() -> erlang:system_flag(internal_cpu_topology, id([foo])) end),
     catch_error(fun() -> erlang:system_flag(internal_cpu_topology, id([{foo, 1}])) end),
     catch_error(fun() -> erlang:system_flag(internal_cpu_topology, id([{cpu, 0, 0, -1, 0, 0, 0} | x])) end),
     catch_error(fun() -> erlang:system_flag(erts_alloc, id(foo)) end),
     catch_error(fun() -> erlang:system_flag(erts_alloc, id({a, b})) end),
     catch_error(fun() -> erlang:system_flag(erts_alloc, id({a, b, c})) end)].

%% `process_flag(message_queue_data, Value)` (bif.c:2167-2172,
%% erl_message.c:1022-1100). logger_olp:init/1 (logger_olp.erl:172) sets
%% `off_heap`, and logger_simple_h:adding_handler/1 (logger_simple_h.erl:39-40)
%% spawns with the option `{message_queue_data, off_heap}`. The flag gives the
%% old value, and the start value is `on_heap`. The last spawn option wins
%% (erl_process.c:12305-12319). Each other value raises badarg.
%% erlang:spawn_opt/2 (erlang.erl:10751-10757) catches the badarg of a spawn
%% option and raises it again with a stack trace, so the badarg case calls
%% erlang:spawn_opt/4.
message_queue_data() ->
    in_new_process(fun() ->
        Self = self(),
        {Pid, Ref} = spawn_opt(fun() -> Self ! {self(), process_flag(message_queue_data, on_heap)} end,
                               [link, monitor, {message_queue_data, off_heap}]),
        Child = receive {Pid, Old} -> Old end,
        receive {'DOWN', Ref, process, Pid, normal} -> ok end,
        {Pid2, Ref2} = spawn_opt(fun() -> Self ! {self(), process_flag(message_queue_data, on_heap)} end,
                                 [monitor, {message_queue_data, on_heap}, {message_queue_data, off_heap}]),
        Child2 = receive {Pid2, Old2} -> Old2 end,
        receive {'DOWN', Ref2, process, Pid2, normal} -> ok end,
        [process_flag(message_queue_data, off_heap),
         process_flag(message_queue_data, off_heap),
         process_flag(message_queue_data, on_heap),
         Child, Child2,
         catch_error(fun() -> process_flag(message_queue_data, id(bad)) end),
         catch_error(fun() -> spawn_opt(erlang, apply, [fun() -> ok end, []], [{message_queue_data, id(bad)}]) end)]
    end).

%% `erlang:process_flag/2` (bif.c:2028-2237). `trap_exit` with a value that is
%% not a boolean, and `message_queue_data` with a value that is not an atom,
%% raise badarg. A flag that ERTS does not know raises badarg,
%% also a tuple that is not `{monitor_nodes, Options}`. A flag that needs a
%% subsystem that PBeam does not have also raises badarg for a value that
%% ERTS rejects before it uses the subsystem: async_dist, sensitive, and
%% monitor_nodes take a boolean, error_handler an atom, priority low, normal,
%% high, or max, save_calls a small integer from 0 to 10000, and scheduler,
%% fullsweep_after, min_heap_size, and min_bin_vheap_size a small integer of
%% 0 or more.
process_flag() ->
    [catch_error(fun() -> process_flag(trap_exit, id(yes)) end),
     catch_error(fun() -> process_flag(message_queue_data, id(1)) end),
     catch_error(fun() -> process_flag(id(no_such_flag), true) end),
     catch_error(fun() -> process_flag(id({foo, bar}), true) end),
     catch_error(fun() -> process_flag(id({monitor_nodes}), true) end),
     catch_error(fun() -> process_flag(id(1), true) end),
     catch_error(fun() -> process_flag(async_dist, id(yes)) end),
     catch_error(fun() -> process_flag(sensitive, id(1)) end),
     catch_error(fun() -> process_flag(monitor_nodes, id(foo)) end),
     catch_error(fun() -> process_flag(error_handler, id(1)) end),
     catch_error(fun() -> process_flag(priority, id(foo)) end),
     catch_error(fun() -> process_flag(save_calls, id(10001)) end),
     catch_error(fun() -> process_flag(save_calls, id(-1)) end),
     catch_error(fun() -> process_flag(save_calls, id(a)) end),
     catch_error(fun() -> process_flag(scheduler, id(-1)) end),
     catch_error(fun() -> process_flag(scheduler, id(a)) end),
     catch_error(fun() -> process_flag(fullsweep_after, id(-1)) end),
     catch_error(fun() -> process_flag(fullsweep_after, id(1 bsl 59)) end),
     catch_error(fun() -> process_flag(min_heap_size, id(a)) end),
     catch_error(fun() -> process_flag(min_bin_vheap_size, id(-1)) end)].

%% A flag that needs a subsystem that PBeam does not have: the stack trace
%% depth, the time offset, the trace control word, the sequential tracer,
%% sequential trace tokens, process priorities, saved calls, the heap size,
%% and node monitors. ERTS gives 8, volatile, 0, false, true, normal, 0, 233,
%% and 0.
unsupported_flag(backtrace_depth) -> erlang:system_flag(backtrace_depth, 8);
unsupported_flag(time_offset) -> erlang:system_flag(time_offset, finalize);
unsupported_flag(trace_control_word) -> erlang:system_flag(trace_control_word, 0);
unsupported_flag(sequential_tracer) -> erlang:system_flag(sequential_tracer, self());
unsupported_flag(reset_seq_trace) -> erlang:system_flag(reset_seq_trace, foo);
unsupported_flag(priority) -> process_flag(priority, normal);
unsupported_flag(save_calls) -> process_flag(save_calls, 0);
unsupported_flag(min_heap_size) -> process_flag(min_heap_size, 233);
unsupported_flag(monitor_nodes) -> process_flag({monitor_nodes, []}, false).

%% `erlang:error/1` (bif.c:1436-1440) raises class error with its argument
%% as the reason. `erlang:raise/3` (bif.c:1549-1692) raises the class error
%% or exit with Reason. It returns the atom badarg, and does not raise, for a
%% class that is not error, exit, or throw, and for a stack trace that is not
%% a proper list of {Fun, Args}, {Fun, Args, Location}, {M, F, A}, or
%% {M, F, A, Location} with a list Location. gen_server:try_terminate/3
%% (gen_server.erl:2513) raises the exception of a terminate callback again
%% with it.
raise() ->
    F = fun id/1,
    Valid = [{m, f, 1}, {m, f, [x], []}, {m, f, 1, [{line, 1} | x]}, {F, [x]}, {F, [x], [x]},
             {fun lists:map/2, [], []}],
    [catch_class(fun() -> erlang:error(id(foo)) end),
     catch_class(fun() -> erlang:error(id({a, b})) end),
     catch_class(fun() -> erlang:raise(id(error), id(r), id([])) end),
     catch_class(fun() -> erlang:raise(id(exit), id({r}), id(Valid)) end),
     erlang:raise(id(foo), id(r), id([])),
     erlang:raise(id(1), id(r), id([])),
     erlang:raise(id(error), id(r), id(foo)),
     erlang:raise(id(error), id(r), id([{m, f, 1} | x])),
     erlang:raise(id(throw), id(r), id([foo])),
     erlang:raise(id(exit), id(r), id([{m}])),
     erlang:raise(id(error), id(r), id([{m, []}])),
     erlang:raise(id(error), id(r), id([{F, [], x}])),
     erlang:raise(id(error), id(r), id([{1, f, 1}])),
     erlang:raise(id(error), id(r), id([{m, 1, 1, []}])),
     erlang:raise(id(error), id(r), id([{m, f, 1, x}])),
     erlang:raise(id(error), id(r), id([{m, f, 1, [], x}]))].

%% The class throw, which PBeam does not implement. ERTS gives {throw, r}.
raise_throw() ->
    catch_class(fun() -> erlang:raise(id(throw), id(r), id([])) end).

big(N) -> big(N, #{}).

big(0, M) -> M;
big(N, M) -> big(N - 1, M#{N => N}).

drop(N, To, M) when N > To -> M;
drop(N, To, M) -> drop(N + 1, To, maps:remove(N, M)).

in_new_process(Fun) ->
    {Pid, Ref} = spawn_monitor(fun() -> exit({result, Fun()}) end),
    receive {'DOWN', Ref, process, Pid, {result, Result}} -> Result end.

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.

catch_class(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

id(X) -> X.
