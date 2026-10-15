-module(map_elements_probe).
-export([main/0, pick/1, same/1]).

main() ->
    Flat = #{true => t, abs_path => p, 1 => one, [] => nil},
    Hash = #{true => t, abs_path => p, 1 => one, [] => nil, k01 => 1, k02 => 2, k03 => 3, k04 => 4, k05 => 5, k06 => 6, k07 => 7, k08 => 8, k09 => 9, k10 => 10, k11 => 11, k12 => 12, k13 => 13, k14 => 14, k15 => 15, k16 => 16, k17 => 17, k18 => 18, k19 => 19, k20 => 20, k21 => 21, k22 => 22, k23 => 23, k24 => 24, k25 => 25, k26 => 26, k27 => 27, k28 => 28, k29 => 29},
    [?MODULE:pick(Flat), ?MODULE:pick(Flat#{abs_path := q}), ?MODULE:pick(maps:remove(abs_path, Flat)),
     ?MODULE:pick(maps:remove(1, Flat)), ?MODULE:pick(#{}), ?MODULE:pick(Hash), ?MODULE:pick(maps:remove([], Hash)),
     ?MODULE:same(Flat), ?MODULE:same(#{true => t})].

%% The values go to the registers of the map and of the other arguments.
pick(#{true := T, abs_path := P, 1 := One, [] := Nil}) -> {T, P, One, Nil};
pick(_) -> missing.

%% The map stays in its register on the fail path.
same(#{true := T, abs_path := P} = Map) -> {T, P, map_size(Map)};
same(Map) -> {missing, map_size(Map)}.
