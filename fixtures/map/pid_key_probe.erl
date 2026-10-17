-module(pid_key_probe).
-export([flat/0, hash/0, order/0, id/1]).

%% The value of each map operation with a pid key. Each result holds no pid,
%% so a run on the pinned ERTS and a run on PBeam print the same text.
flat() ->
    P1 = id(self()),
    P2 = id(spawn(fun() -> ok end)),
    M = id(#{{a, P1} => 1, {b, P2} => 2, P1 => 3, [P2] => 4}),
    [maps:size(M), map_get({a, P1}, M), map_get(P1, M), map_get([P2], M),
     is_map_key(P1, M), is_map_key(P2, M),
     maps:keys(M) =:= [P1, {a, P1}, {b, P2}, [P2]]].

%% A hashmap of 34 keys that holds one pid key.
hash() ->
    P = id(self()),
    M = id(maps:from_list(pair(1) ++ [{P, pid}])),
    [maps:size(M), map_get(P, M), map_get({i, 17}, M), is_map_key(P, M)].

pair(34) -> [];
pair(N) -> [{id({i, N}), N} | pair(N + 1)].

%% The term order of two pids, alone and inside tuples.
order() ->
    P1 = id(self()),
    P2 = id(spawn(fun() -> ok end)),
    [P1 < P2, min(P2, P1) =:= P1, {a, P1} < {a, P2},
     min({a, P2}, {a, P1}) =:= {a, P1}].

id(Term) -> Term.
