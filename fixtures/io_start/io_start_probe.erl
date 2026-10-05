-module(io_start_probe).
-export([lists/0]).

%% `bif` `erlang:tl/1` with fail label 0, as group:init/1 (group.erl:136)
%% runs it, and in a guard. `get_tl`, as proc_lib:trans_init/3
%% (proc_lib.erl:939-944) runs it. `gc_bif` `erlang:length/1` with fail
%% label 0, as proc_lib:trans_init/3 (proc_lib.erl:944) runs it, and in a
%% guard. A term that is not a list, and the tail of [], raise badarg.
lists() ->
    [tl(id([a, b, c])),
     tl(id([a])),
     try tl(id([])) catch error:R1 -> R1 end,
     try tl(id(foo)) catch error:R2 -> R2 end,
     length(id([])),
     length(id([a, b, c])),
     try length(id(foo)) catch error:R3 -> R3 end,
     try length(id(<<1>>)) catch error:R4 -> R4 end,
     rest(id([x, y, z])),
     tail_guard(id([1, 2])), tail_guard(id([])), tail_guard(id(foo)),
     length_guard(id([1, 2, 3])), length_guard(id(foo))].

rest([_ | Rest]) -> Rest.

tail_guard(L) when tl(L) =:= [2] -> two;
tail_guard(_) -> other.

length_guard(L) when length(L) > 2 -> long;
length_guard(_) -> other.

id(X) -> X.
