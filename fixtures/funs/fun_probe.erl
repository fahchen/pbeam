-module(fun_probe).
-export([mfas/0, locals/0, tests/0, applies/0, spawned/0, externals/0, infos/0, fun_info_unsupported/1, texts/0, tail_wait/1, id/1]).

%% erlang:apply/3 and the apply instruction, as proc_lib:init_p_do_apply/3
%% runs them. apply/3 takes at most 1023 arguments.
mfas() ->
    M = id(fun_probe), F = id(id),
    [apply(fun_probe, id, id([v])),
     apply(erlang, apply, id([fun_probe, id, [w]])),
     M:F(z),
     (id(erlang)):(id(apply))(fun_probe, id, [y]),
     tail_apply(fun_probe, id, [s]),
     try apply(id(1), id, []) catch error:R1 -> R1 end,
     try apply(fun_probe, id(1), []) catch error:R2 -> R2 end,
     try apply(id(1), id, id([])) catch error:R3 -> R3 end,
     try apply(fun_probe, id(1), id([])) catch error:R4 -> R4 end,
     try apply(fun_probe, id, id(notalist)) catch error:R5 -> R5 end,
     try apply(erlang, apply, id([fun_probe, id(1), []])) catch error:R6 -> R6 end,
     try (id(1)):F(z) catch error:R7 -> R7 end,
     try M:(id(1))(z) catch error:R8 -> R8 end,
     try apply(fun_probe, id, [x, x | list1022()]) catch error:R9 -> R9 end].

%% make_fun3 with and without free variables, call_fun and call_fun2 with
%% each tag, fun equality and order, and the badarity and badfun exceptions.
locals() ->
    Tag = id(tag),
    Pair = fun(X) -> {Tag, X} end,
    Const = fun() -> const end,
    [Pair(one),
     (id(Const))(),
     tail_call(Pair),
     eq(make_pair(a), make_pair(a)),
     eq(make_pair(a), make_pair(b)),
     eq(Const, id(Const)),
     make_pair(b) >= make_pair(a),
     make_pair(a) >= make_pair(b),
     try (id(Pair))(1, 2) catch error:{badarity, {F, Args}} -> {badarity, eq(F, Pair), Args} end,
     try (id(notfun))() catch error:R1 -> R1 end,
     try (id({fun_probe, id}))(x) catch error:R2 -> R2 end].

%% The is_function and is_function2 tests, lists:foreach/2, and the BIFs
%% is_function/1,2.
tests() ->
    Pair = make_pair(a),
    Self = self(),
    ok = lists:foreach(fun(X) -> Self ! {item, X} end, [a, b]),
    Items = [receive {item, a} -> a end, receive {item, b} -> b end],
    [Items,
     kind(Pair), kind(fun() -> ok end), kind(notfun),
     is_function(id(Pair)), is_function(id(notfun)),
     is_function(id(Pair), id(1)), is_function(id(Pair), id(0)),
     is_function(id(notfun), id(1)),
     arity_guard(Pair, id(1)), arity_guard(Pair, id(-1)), arity_guard(Pair, id(a)),
     try is_function(id(Pair), id(-1)) catch error:R1 -> R1 end,
     try is_function(id(notfun), id(a)) catch error:R2 -> R2 end].

%% erlang:apply/2 with a fun. apply/2 takes at most 1022 arguments on the JIT.
applies() ->
    Pair = make_pair(a),
    L1022 = list1022(),
    [apply(Pair, [x]),
     apply(erlang, apply, [Pair, [u]]),
     tail_apply(Pair, [t]),
     framed_apply(Pair, [r]),
     try apply(Pair, id(notalist)) catch error:R1 -> R1 end,
     try apply(id(notfun), []) catch error:R2 -> R2 end,
     try apply(id(notfun), id(notalist)) catch error:R3 -> R3 end,
     try apply(Pair, [1, 2]) catch error:{badarity, {F1, A1}} -> {badarity, eq(F1, Pair), A1} end,
     try apply(Pair, L1022) catch error:{badarity, {F2, A2}} -> {badarity, eq(F2, Pair), eq(A2, L1022)} end,
     try apply(Pair, [x | L1022]) catch error:R4 -> R4 end].

%% spawn/1 starts a process at erlang:apply/2.
spawned() ->
    Self = self(),
    Tag = id(spawned),
    Pid = spawn(fun() -> Self ! {Tag, self()} end),
    Reply = receive {spawned, Pid} -> spawned end,
    [Reply, try spawn(id(notfun)) catch error:R -> R end].

%% erlang:make_fun/3 makes an external fun, as gen_server:init_it/6 does. An
%% external fun literal loads from the literal table, as in group:init/3.
externals() ->
    M = id(fun_probe), F = id(id),
    Ext = fun M:F/1,
    Pair = make_pair(a),
    [Ext(v), apply(Ext, [w]), eq((erlang:make_fun(id(erlang), id(self), 0))(), self()),
     eq(Ext, erlang:make_fun(M, F, id(1))),
     eq(Ext, erlang:make_fun(M, F, id(2))),
     Ext >= Pair, Pair >= Ext,
     erlang:make_fun(M, id(mfas), id(0)) >= Ext,
     kind(Ext),
     try (id(Ext))(1, 2) catch error:{badarity, {F1, A1}} -> {badarity, eq(F1, Ext), A1} end,
     try erlang:make_fun(id(1), F, 1) catch error:R1 -> R1 end,
     try erlang:make_fun(M, id("id"), 1) catch error:R2 -> R2 end,
     try erlang:make_fun(M, F, id(-1)) catch error:R3 -> R3 end,
     try erlang:make_fun(M, F, id(256)) catch error:R4 -> R4 end,
     try erlang:make_fun(M, F, id(a)) catch error:R5 -> R5 end,
     eq(erlang:make_fun(M, F, id(255)), erlang:make_fun(M, F, id(255))),
     (id(fun fun_probe:id/1))(lit), eq(Ext, fun fun_probe:id/1)].

%% erlang:fun_info/2 on a local fun with free variables, a local fun without
%% free variables, and an external fun, as Elixir.BadArityError asks for the
%% arity of a fun and Elixir.Agent.Server asks for its module and name.
infos() ->
    Tag = {id(a), id(b)},
    K = id(7),
    Pair = fun(X) -> {Tag, K, X} end,
    Const = fun() -> const end,
    Ext = fun lists:reverse/1,
    [erlang:fun_info(Pair, I) || I <- [type, module, name, arity, env, refc, uniq, new_index]] ++
    [erlang:fun_info(Const, I) || I <- [type, module, name, arity, env, refc,
                                       uniq, new_index]] ++
    [erlang:fun_info(Ext, I) || I <- [type, module, name, arity, env, refc, index,
                              new_index, uniq, new_uniq, pid]] ++
    [try erlang:fun_info(id(42), type) catch error:R1 -> R1 end,
     try erlang:fun_info(Pair, id(bogus)) catch error:R2 -> R2 end,
     try erlang:fun_info(Pair, native) catch error:R3 -> R3 end,
     try erlang:fun_info(Ext, native_address) catch error:R4 -> R4 end].

%% The fun_info/2 items that PBeam refuses with a runtime error because the
%% loader keeps no such value.
fun_info_unsupported(Item) ->
    Tag = id(tag),
    Pair = fun(X) -> {Tag, X} end,
    erlang:fun_info(Pair, Item).

%% A fun call in a tail position replaces the frame of its caller, as the
%% ERTS loader makes i_call_fun_last of call_fun, deallocate, and return.
tail_wait(Tag) ->
    F = id(fun() -> receive Tag -> Tag end end),
    _ = id(x),
    F().

%% A list of 1022 atoms x, built without arithmetic.
list1022() ->
    L1 = id([x]), L2 = L1 ++ L1, L4 = L2 ++ L2, L8 = L4 ++ L4, L16 = L8 ++ L8,
    L32 = L16 ++ L16, L64 = L32 ++ L32, L128 = L64 ++ L64, L256 = L128 ++ L128,
    L512 = L256 ++ L256,
    L512 ++ L256 ++ L128 ++ L64 ++ L32 ++ L16 ++ L8 ++ L4 ++ L2.

make_pair(T) -> fun(X) -> {T, X} end.
tail_call(F) -> F(tail).
tail_apply(F, A) -> apply(F, A).
tail_apply(M, F, A) -> apply(M, F, A).
framed_apply(F, A) -> _ = id(x), apply(F, A).
eq(X, X) -> same;
eq(_, _) -> different.
kind(F) when is_function(F, 1) -> one;
kind(F) when is_function(F) -> any;
kind(_) -> none.
arity_guard(F, N) when is_function(F, N) -> true;
arity_guard(_, _) -> false.
id(X) -> X.

%% erlang:fun_to_list/1 and io_lib ~p and ~w of a local fun, a local fun
%% without free variables, external funs (one with quoted atoms, one of a
%% module that is not loaded), and a term that is not a fun.
texts() ->
    K = id(7),
    Pair = fun(X) -> {K, X} end,
    Const = fun() -> const end,
    Ext = fun lists:reverse/1,
    Quoted = fun 'A b':'c d'/0,
    Missing = fun no_such_module:f/3,
    [erlang:fun_to_list(F) || F <- [Pair, Const, Ext, Quoted, Missing]] ++
    [lists:flatten(io_lib:format("~p ~w", [F, F])) || F <- [Pair, Ext, Quoted]] ++
    [try erlang:fun_to_list(id(42)) catch error:R -> R end].
