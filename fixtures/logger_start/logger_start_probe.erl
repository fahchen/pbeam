-module(logger_start_probe).
-export([reverse/0, remove/0, without/0]).

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

big(N) -> big(N, #{}).

big(0, M) -> M;
big(N, M) -> big(N - 1, M#{N => N}).

drop(N, To, M) when N > To -> M;
drop(N, To, M) -> drop(N + 1, To, maps:remove(N, M)).

id(X) -> X.
