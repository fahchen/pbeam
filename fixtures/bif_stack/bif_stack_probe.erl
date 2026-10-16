-module(bif_stack_probe).
-export([main/0, uncaught/0, hibernated/0]).

%% The fixture has no line table, so ERTS gives each frame of this module
%% the location []. stack/1 drops the frames of the erl -eval caller.
main() ->
    [guard_bifs(), gc_bifs(), call_ext(), tail_calls(), applies(), funs(),
     initial_calls(), spawn_opt_badarg(), length_trap()].

id(X) -> X.

%% A bif instruction with the fail label 0.
guard_bifs() ->
    [try_stack(fun() -> element(5, id({a})) end),
     try_stack(fun() -> hd(id([])) end),
     try_stack(fun() -> tl(id(a)) end),
     try_stack(fun() -> tuple_size(id(a)) end),
     try_stack(fun() -> map_get(k, id(#{})) end),
     try_stack(fun() -> is_map_key(k, id(a)) end)].

%% A gc_bif instruction with the fail label 0.
gc_bifs() ->
    [try_stack(fun() -> length(id(a)) end),
     try_stack(fun() -> byte_size(id(a)) end),
     try_stack(fun() -> abs(id(a)) end),
     try_stack(fun() -> id(a) + 1 end),
     try_stack(fun() -> 1 div id(0) end),
     try_stack(fun() -> -id(a) end),
     try_stack(fun() -> bnot id(a) end),
     try_stack(fun() -> float(id(a)) end)].

%% A call_ext of a BIF in a body position.
call_ext() ->
    [try_stack(fun() -> body(atom_to_list, 1) end),
     try_stack(fun() -> body(list_to_atom, 1) end),
     try_stack(fun() -> body(integer_to_list, a) end),
     try_stack(fun() -> body(list_to_tuple, a) end),
     try_stack(fun() -> body(member, a) end),
     try_stack(fun() -> body(reverse, a) end),
     try_stack(fun() -> body(get, a) end),
     try_stack(fun() -> body(system_time, a) end)].

body(atom_to_list, X) -> _ = atom_to_list(X), ok;
body(list_to_atom, X) -> _ = list_to_atom(X), ok;
body(integer_to_list, X) -> _ = integer_to_list(X), ok;
body(list_to_tuple, X) -> _ = list_to_tuple(X), ok;
body(member, X) -> _ = lists:member(x, X), ok;
body(reverse, X) -> _ = lists:reverse(X, []), ok;
body(get, X) -> _ = maps:get(k, X), ok;
body(system_time, X) -> _ = os:system_time(X), ok.

%% A call_ext_only and a call_ext_last of a BIF.
tail_calls() ->
    [try_stack(fun() -> only(1) end),
     try_stack(fun() -> last(1) end),
     try_stack(fun() -> only_lists(a) end)].

only(X) -> atom_to_list(X).

last(X) ->
    Y = id(X),
    _ = id(Y),
    tuple_to_list(Y).

only_lists(X) -> lists:keyfind(k, 1, X).

%% apply/2, apply/3, apply_last, and erlang:apply/3 of a BIF.
applies() ->
    [try_stack(fun() -> apply_body(id(erlang), id(atom_to_list), [1]) end),
     try_stack(fun() -> apply_tail(id(erlang), id(atom_to_list), [1]) end),
     try_stack(fun() -> apply_fun(fun erlang:atom_to_list/1, [1]) end),
     try_stack(fun() -> apply_body(id(lists), id(member), [x, a]) end),
     try_stack(fun() -> erlang:apply(id(1), f, []) end),
     try_stack(fun() -> apply_body(id(1), id(f), []) end),
     try_stack(fun() -> apply_fun(fun erlang:atom_to_list/1, a) end),
     try_stack(fun() -> dynamic(id(1), f) end)].

dynamic(M, F) ->
    _ = M:F(a),
    ok.

apply_body(M, F, A) ->
    _ = apply(M, F, A),
    ok.

apply_tail(M, F, A) ->
    apply(M, F, A).

apply_fun(F, A) ->
    _ = apply(F, A),
    ok.

%% A body call and a tail call of a fun of a BIF.
funs() ->
    [try_stack(fun() -> fun_body(id(fun erlang:atom_to_list/1)) end),
     try_stack(fun() -> fun_tail(id(fun erlang:atom_to_list/1)) end),
     try_stack(fun() -> fun_body(id(fun erlang:tuple_size/1)) end)].

fun_body(F) ->
    _ = F(1),
    ok.

fun_tail(F) ->
    F(1).

%% The initial call of a process is a BIF, or erlang:apply/3 of a BIF.
initial_calls() ->
    [down(erlang, atom_to_list, [1]),
     down(erlang, apply, [erlang, atom_to_list, [1]]),
     down(erlang, apply, [fun erlang:atom_to_list/1, [1]]),
     down(lists, member, [x, a]),
     down(?MODULE, hibernated, [])].

down(M, F, A) ->
    {Pid, Ref} = spawn_monitor(M, F, A),
    Pid ! wake,
    receive {'DOWN', Ref, process, _, Reason} -> Reason end.

%% The wake-up applies erlang:apply(1, f, []).
hibernated() ->
    erlang:hibernate(erlang, apply, [1, f, []]).

%% spawn_opt/2 with a bad option gets the stack trace of the BIF. The
%% first frame holds the arity, as the text of a fun differs.
spawn_opt_badarg() ->
    {C, R, [{M, F, A, L} | S]} =
        try_stack(fun() -> spawn_opt(fun() -> ok end, [{monitor, foo}]) end),
    {C, R, [{M, F, length(A), L} | S]}.



%% The root process exits with the result of main/0, so the run gives its
%% text.
uncaught() ->
    exit(main()).

try_stack(F) ->
    try F() catch C:R:S -> {C, R, stack(S)} end.

stack([{M, _, _, _} = Frame | S]) when M =/= erl_eval, M =/= init -> [Frame | stack(S)];
stack(_) -> [].

%% length/1 traps on a long list and raises with the list of the call
%% (erl_bif_guard.c:243-247). The first frame holds the length of that
%% list.
length_trap() ->
    L = lists:seq(1, 100000) ++ a,
    {C, R, [{M, F, [List], Loc} | S]} = try_stack(fun() -> length(id(L)) end),
    {C, R, [{M, F, [count(List)], Loc} | S]}.

count([_ | T]) -> 1 + count(T);
count(_) -> 0.
