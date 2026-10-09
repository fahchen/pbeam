-module(map_size_probe).
-export([main/0, body/1, guard/1, id/1]).

main() ->
    Flat = ?MODULE:id(#{<<"a">> => 0, <<"b">> => 0}),
    Hash = ?MODULE:id(#{<<"k01">> => 1, <<"k02">> => 2, <<"k03">> => 3, <<"k04">> => 4, <<"k05">> => 5, <<"k06">> => 6, <<"k07">> => 7, <<"k08">> => 8, <<"k09">> => 9, <<"k10">> => 10, <<"k11">> => 11, <<"k12">> => 12, <<"k13">> => 13, <<"k14">> => 14, <<"k15">> => 15, <<"k16">> => 16, <<"k17">> => 17, <<"k18">> => 18, <<"k19">> => 19, <<"k20">> => 20, <<"k21">> => 21, <<"k22">> => 22, <<"k23">> => 23, <<"k24">> => 24, <<"k25">> => 25, <<"k26">> => 26, <<"k27">> => 27, <<"k28">> => 28, <<"k29">> => 29, <<"k30">> => 30, <<"k31">> => 31, <<"k32">> => 32, <<"k33">> => 33}),
    [map_size(#{<<"a">> => 0, <<"b">> => 0}), map_size(Flat), map_size(?MODULE:id(#{})), map_size(Hash), map_size(Hash#{<<"k34">> => 34}),
     ?MODULE:guard(Flat), ?MODULE:guard(Hash), ?MODULE:guard(#{}), ?MODULE:guard(foo), ?MODULE:guard([1]),
     ?MODULE:body(foo), ?MODULE:body([1]), ?MODULE:body(<<"ab">>)].

body(Term) ->
    try map_size(Term) catch Class:Reason -> {Class, Reason} end.

guard(Term) when map_size(Term) > 2 -> big;
guard(Term) when map_size(Term) >= 0 -> small;
guard(_) -> other.

id(Term) -> Term.
