-module(logger_start_probe).
-export([reverse/0, remove/0, without/0, ets/0, ets_access/0]).

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

big(N) -> big(N, #{}).

big(0, M) -> M;
big(N, M) -> big(N - 1, M#{N => N}).

drop(N, To, M) when N > To -> M;
drop(N, To, M) -> drop(N + 1, To, maps:remove(N, M)).

catch_error(Fun) ->
    try Fun() catch error:Reason -> {error, Reason} end.

id(X) -> X.
