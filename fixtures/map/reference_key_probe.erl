-module(reference_key_probe).
-export([flat/0, hash/0, order/0, id/1]).

%% The value of each map operation with a reference key. Each result holds
%% no reference, so a run on the pinned ERTS and a run on PBeam print the
%% same text.
flat() ->
    R1 = id(make_ref()),
    R2 = id(make_ref()),
    M = id(#{{a, R1} => 1, {b, R2} => 2, R1 => 3, [R2] => 4}),
    [maps:size(M), map_get({a, R1}, M), map_get(R1, M), map_get([R2], M),
     is_map_key(R1, M), is_map_key(R2, M), maps:keys(M) =:= [R1, {a, R1}, {b, R2}, [R2]]].

%% A hashmap of 34 keys that holds one reference key.
hash() ->
    R = id(make_ref()),
    M = id(maps:from_list(pair(1) ++ [{R, ref}])),
    [maps:size(M), map_get(R, M), map_get({i, 17}, M), is_map_key(R, M)].

pair(34) -> [];
pair(N) -> [{id({i, N}), N} | pair(N + 1)].

%% The term order of two references, alone and inside tuples.
order() ->
    R1 = id(make_ref()),
    R2 = id(make_ref()),
    [R1 < R2, min(R2, R1) =:= R1, {a, R1} < {a, R2},
     min({a, R2}, {a, R1}) =:= {a, R1}].

id(Term) -> Term.
