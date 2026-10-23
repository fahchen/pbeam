-module(port_key_probe).
-export([flat/0, hash/0, high/0, nested/0, order/0, id/1]).

%% The value of each map operation with a port key. Each port is the local
%% port of a fixed number, made from its external format, so a run on the
%% pinned ERTS and a run on PBeam use the same ports. Each result holds no
%% port.
port(N) ->
    binary_to_term(<<131, 89, 119, 13, "nonode@nohost", N:32, 0:32>>).

flat() ->
    P1 = id(port(1)), P2 = id(port(2)),
    M = id(#{P1 => 1, P2 => 2, a => 3, 7 => 4, {P1} => 5, [P2] => 6,
             self() => 7, <<"b">> => 8}),
    M2 = maps:remove(port(2), M),
    [maps:size(M), map_get(port(1), M), map_get({port(1)}, M),
     map_get([port(2)], M), is_map_key(port(3), M), is_map_key(P2, M2),
     maps:size(M2), [V || {_, V} <- maps:to_list(M)],
     [V || {_, V} <- maps:to_list(M2)]].

%% A hashmap of 40 port keys: the pair order follows the internal hash.
hash() ->
    M = id(maps:from_list([{port(N), N} || N <- lists:seq(1, 40)])),
    M2 = maps:remove(port(17), M),
    [maps:size(M), map_get(port(17), M), is_map_key(port(41), M),
     is_map_key(port(17), M2), [V || {_, V} <- maps:to_list(M)],
     [V || {_, V} <- maps:to_list(M2)]].

%% A hashmap of 40 port keys from 8190: a port number of 8192 or more uses
%% the high bits of the port table index.
high() ->
    M = id(maps:from_list([{port(N), N} || N <- lists:seq(8190, 8229)])),
    [V || {_, V} <- maps:to_list(M)].

%% A hashmap of port keys inside tuples, lists, and map keys.
nested() ->
    M = id(maps:from_list([{{N, port(N)}, N} || N <- lists:seq(1, 34)]
                          ++ [{[port(N)], -N} || N <- lists:seq(1, 4)]
                          ++ [{#{port(N) => N}, 100 + N} || N <- [1, 2]])),
    [maps:size(M), map_get([port(2)], M), map_get({5, port(5)}, M),
     map_get(#{port(2) => 2}, M), maps:values(M)].

%% The term order of ports.
order() ->
    [port(1) < port(2), port(2) < port(1), port(1) =:= port(1),
     port(1) == port(1), port(1) > self(), port(1) < self(),
     port(1) > make_ref(),
     lists:sort([port(3), port(1), port(2)]) =:= [port(1), port(2), port(3)]].

id(T) -> T.
