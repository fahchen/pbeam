-module(pid_order_probe).
-export([hash/0, id/1]).

%% A hashmap of 40 pid keys: the pair order follows the internal hash. Each
%% pid is the local pid of a fixed number, made from its external format, so
%% a run on the pinned ERTS and a run on PBeam use the same pids. The result
%% holds no pid.
pid(N) ->
    binary_to_term(<<131, 88, 119, 13, "nonode@nohost", N:32, 0:32, 0:32>>).

hash() ->
    M = id(maps:from_list([{pid(N), N} || N <- lists:seq(1, 40)])),
    M2 = maps:remove(pid(17), M),
    [maps:size(M), map_get(pid(17), M), is_map_key(pid(41), M),
     is_map_key(pid(17), M2), [V || {_, V} <- maps:to_list(M)],
     [V || {_, V} <- maps:to_list(M2)]].

id(T) -> T.
