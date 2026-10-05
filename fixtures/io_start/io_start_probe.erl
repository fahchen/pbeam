-module(io_start_probe).
-export([lists/0, maps/0, merge/0, atoms/0, atom_limit/0, system/0,
         system_other/0, format/0]).

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

%% `bif` `erlang:map_get/2` with fail label 0, as prim_tty:init/1
%% (prim_tty.erl:220) and user_drv:init/1 (user_drv.erl:202) run it, and in
%% a guard. `put_map_exact` with fail label 0, as
%% io_lib_format:count_small/2 (io_lib_format.erl:325) runs it, and in a
%% guard. With two missing keys, ERTS reports the first key in the flatmap
%% key order (atom index): `value` before `abs`.
maps() ->
    M = id(#{input => cooked, output => raw}),
    [map_get(input, M),
     try map_get(ofd, M) catch error:R1 -> R1 end,
     try map_get(input, id(foo)) catch error:R2 -> R2 end,
     get_guard(M), get_guard(id(#{})), get_guard(id(foo)),
     M#{output := cooked},
     try (id(#{}))#{output := cooked} catch error:R3 -> R3 end,
     try (id(#{}))#{abs := 1, value := 2} catch error:R4 -> R4 end,
     update_guard(M), update_guard(id(#{input => cooked})), update_guard(id(foo))].

get_guard(M) when map_get(input, M) =:= cooked -> cooked;
get_guard(_) -> other.

update_guard(M) when M#{output := cooked} =:= #{input => cooked, output => cooked} -> updated;
update_guard(_) -> other.

%% `maps:merge/2`, as prim_tty:options/1 (prim_tty.erl:329) runs it: the
%% values of the second map win. A first argument that is not a map raises
%% {badmap, First}, else a second argument that is not a map raises
%% {badmap, Second}.
merge() ->
    M = id(#{input => cooked, output => raw}),
    [maps:merge(id(#{input => raw, output => cooked, ofd => stdout}), M),
     maps:merge(M, id(#{})),
     maps:merge(M, M),
     maps:merge(id(big(33)), id(#{1 => one, 40 => forty})),
     try maps:merge(id(foo), M) catch error:R1 -> R1 end,
     try maps:merge(M, id(bar)) catch error:R2 -> R2 end,
     try maps:merge(id(foo), id(bar)) catch error:R3 -> R3 end].

big(N) -> big(N, #{}).
big(0, M) -> M;
big(N, M) -> big(N - 1, M#{N => N}).

%% `erlang:atom_to_list/1` and `erlang:list_to_atom/1`, as
%% prim_tty:set_name/2 (prim_tty.erl:569-571) runs them. A term that is not
%% an atom, and a list element that is not a Unicode scalar value, raise
%% badarg.
atoms() ->
    [atom_to_list(id(user_drv)),
     atom_to_list(id('')),
     atom_to_list(id('héllo')),
     try atom_to_list(id("user")) catch error:R1 -> R1 end,
     list_to_atom(id("user_drv_writer")) =:= user_drv_writer,
     list_to_atom(id("")),
     atom_to_list(list_to_atom([104, 233, id(1024), id(128512)])),
     try list_to_atom(id(foo)) catch error:R2 -> R2 end,
     try list_to_atom(id([a])) catch error:R3 -> R3 end,
     try list_to_atom([id(-1)]) catch error:R4 -> R4 end,
     try list_to_atom([id(16#D800)]) catch error:R5 -> R5 end,
     try list_to_atom([id(16#110000)]) catch error:R6 -> R6 end,
     try list_to_atom(id(<<"ok">>)) catch error:R7 -> R7 end].

%% `erlang:list_to_atom/1` counts each element before it checks it
%% (utils.c:2862-2930): 255 characters make an atom, and a 256th element
%% raises system_limit, also when it is not a character.
atom_limit() ->
    A = list_to_atom(dup(255, 16#1F600)),
    [length(atom_to_list(A)),
     try list_to_atom(dup(256, $a)) catch error:R1 -> R1 end,
     try list_to_atom(dup(255, $a) ++ [foo]) catch error:R2 -> R2 end,
     try list_to_atom([foo | dup(255, $a)]) catch error:R3 -> R3 end].

dup(0, _) -> [];
dup(N, C) -> [C | dup(N - 1, C)].

%% `os:type/0` (os.erl:383-384), as prim_tty:init_term/1 (prim_tty.erl:284)
%% runs it, and `erlang:system_info(os_type)` directly.
system() ->
    [os:type(), erlang:system_info(id(os_type))].

%% A `erlang:system_info/1` item that the start path does not use.
system_other() ->
    erlang:system_info(id(otp_release)).

%% `io_lib:bformat/2`, as user_drv:init_shell/2 (user_drv.erl:374) runs it
%% with the empty slogan of `-noshell` (user_drv.erl:271-273).
format() ->
    io_lib:bformat(id("~ts"), [id("")]).

id(X) -> X.
