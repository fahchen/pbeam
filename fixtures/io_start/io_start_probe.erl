-module(io_start_probe).
-export([lists/0, maps/0, map_keys/0, merge/0, atoms/0, atom_limit/0, system/0,
         system_other/0, otp_release/0, endian/0, format/0, nots/0,
         keymembers/0, keysearches/0, ands/0, ors/0]).

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

%% `put_map_exact` with keys of each type. ERTS sorts the keys of more than
%% one pair with erts_cmp_flatmap_keys (erl_utils.h:266-271): two atoms
%% compare by atom index, each other two keys in the term order, so the
%% atoms in a tuple compare by name. A key of a type that a PBeam map cannot
%% hold is a missing key.
map_keys() ->
    M = id(#{a => 1}),
    [try M#{id({x}) := 2} catch error:R1 -> R1 end,
     try M#{{x} := 2} catch error:R2 -> R2 end,
     try M#{{x} := 2, b := 3} catch error:R3 -> R3 end,
     try M#{a := 2, {x} := 3} catch error:R4 -> R4 end,
     try M#{{x} := 1, b := 2, 1 := 3} catch error:R5 -> R5 end,
     try M#{[a] := 1, {x} := 2} catch error:R6 -> R6 end,
     try M#{<<"b">> := 1, {x} := 2} catch error:R7 -> R7 end,
     try M#{{value} := 1, {abs} := 2} catch error:R8 -> R8 end,
     try M#{#{} := 1, {} := 2, [] := 3} catch error:R9 -> R9 end,
     key_guard(M, id({x})), key_guard(M, id(a))].

key_guard(M, K) when M#{K := 2} =:= #{a => 2} -> updated;
key_guard(_, _) -> other.

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

%% `erlang:system_info(otp_release)`, as elixir:parse_otp_release/0
%% (elixir.erl:127-129 of Elixir v1.20.4) runs it.
otp_release() ->
    erlang:system_info(id(otp_release)).

%% `erlang:system_info(endian)`, as elixir:check_endianness/0
%% (elixir.erl:33-41 of Elixir v1.20.4) runs it.
endian() ->
    erlang:system_info(id(endian)).

%% A `erlang:system_info/1` item that PBeam does not support.
system_other() ->
    erlang:system_info(id(wordsize)).

%% `io_lib:bformat/2`, as user_drv:init_shell/2 (user_drv.erl:374) runs it
%% with the empty slogan of `-noshell` (user_drv.erl:271-273).
format() ->
    io_lib:bformat(id("~ts"), [id("")]).

%% `erlang:not/1` as the `bif` instruction with fail label 0, as
%% group:init/1 (group.erl:149, 153) runs it, through the `apply`
%% instruction, and in a guard, where the `bif` instruction has a fail
%% label. A term that is not a boolean raises badarg (erl_bif_op.c:84-91);
%% in a guard it jumps to the fail label.
nots() ->
    [not id(true), not id(false),
     try not id(foo) catch error:R1 -> R1 end,
     try not id(1) catch error:R2 -> R2 end,
     apply(erlang, id('not'), [id(true)]),
     try apply(erlang, id('not'), [id([])]) catch error:R3 -> R3 end,
     not_guard(id(true), id(false)), not_guard(id(true), id(true)),
     not_guard(id(foo), id(true))].

not_guard(X, Y) when (not X) =:= Y -> same;
not_guard(_, _) -> other.

%% `lists:keymember/3` with `call_ext`, as user_sup:get_user/1
%% (user_sup.erl:121-122) runs it, and through the `apply` instruction. It
%% runs keyfind (erl_bif_lists.c:1265-1277, 1301-1380): an element that is
%% not a tuple, or a tuple with fewer than N elements, does not match. A
%% position that is not a small integer of at least 1, and a third argument
%% that is not a list, raise badarg.
keymembers() ->
    L = id([{a, 1}, b, {}, {c}, {d, e, f}, {[x], 2}, {{y}, 3}]),
    [lists:keymember(a, 1, L), lists:keymember(1, 2, L), lists:keymember(c, 1, L),
     lists:keymember(f, 3, L), lists:keymember(f, 4, L), lists:keymember(b, 1, L),
     lists:keymember([x], 1, L), lists:keymember({y}, 1, L), lists:keymember(z, 1, L),
     lists:keymember(a, 1, id([])),
     try lists:keymember(a, id(0), L) catch error:R1 -> R1 end,
     try lists:keymember(a, id(-1), L) catch error:R2 -> R2 end,
     try lists:keymember(a, id(foo), L) catch error:R3 -> R3 end,
     try lists:keymember(a, id(1 bsl 60), L) catch error:R4 -> R4 end,
     try lists:keymember(a, 1, id(foo)) catch error:R5 -> R5 end,
     try lists:keymember(a, id(0), id(foo)) catch error:R6 -> R6 end,
     apply(lists, id(keymember), [d, 1, L])].

%% lists:keysearch/3, which inet:pi_replace/2 (inet.erl:2721) calls, with
%% call_ext and through the apply instruction. It runs keyfind and gives
%% {value, Tuple} for a found tuple (erl_bif_lists.c:1279-1292).
keysearches() ->
    L = id([{a, 1}, b, {c}, {1.0, f}]),
    [lists:keysearch(a, 1, L), lists:keysearch(1, 1, L), lists:keysearch(b, 1, L),
     lists:keysearch(z, 1, L), lists:keysearch(c, 1, id([])),
     try lists:keysearch(a, id(0), L) catch error:R1 -> R1 end,
     try lists:keysearch(a, 1, id(foo)) catch error:R2 -> R2 end,
     apply(lists, id(keysearch), [c, 1, L])].

id(X) -> X.

%% `bif` `erlang:and/2` with fail label 0, in a guard with a fail label, as
%% group:io_request/5 (group.erl:250) runs it, and through the `apply`
%% instruction. Two booleans give their conjunction; each other term raises
%% badarg (erl_bif_op.c:45-56), and in a guard jumps to the fail label.
ands() ->
    [id(true) and id(true), id(true) and id(false), id(false) and id(true),
     id(false) and id(false),
     try id(true) and id(foo) catch error:R1 -> R1 end,
     try id(foo) and id(false) catch error:R2 -> R2 end,
     try id(false) and id(1) catch error:R3 -> R3 end,
     apply(erlang, id('and'), [id(true), id(true)]),
     try apply(erlang, id('and'), [id(false), id([])]) catch error:R4 -> R4 end,
     and_guard(id(true)), and_guard(id(false)), and_guard(id(foo))].

and_guard(X) when (X and true) =:= false -> false;
and_guard(_) -> other.

%% `bif` `erlang:or/2` and `erlang:xor/2` with fail label 0, and through the
%% `apply` instruction, as the kernel start calls `erlang:or/2`. Two booleans
%% give the result; each other term raises badarg, and both arguments are
%% evaluated (erl_bif_op.c:58-82).
ors() ->
    [[id(L) or id(R) || L <- [true, false], R <- [true, false]],
     [id(L) xor id(R) || L <- [true, false], R <- [true, false]],
     [try id(L) or id(R) catch error:E1 -> E1 end
      || {L, R} <- [{true, foo}, {foo, true}, {false, 1}, {[], false}]],
     [try id(L) xor id(R) catch error:E2 -> E2 end
      || {L, R} <- [{true, foo}, {foo, false}]],
     apply(erlang, id('or'), [id(false), id(true)]),
     apply(erlang, id('xor'), [id(true), id(true)]),
     try apply(erlang, id('or'), [id(true), id(x)]) catch error:E3 -> E3 end].
