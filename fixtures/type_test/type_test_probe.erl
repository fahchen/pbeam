-module(type_test_probe).
-export([bif/0, funs/0, applies/0, guard/0, arity/0, record/0,
         record_guard/0]).

%% One term of each kind. 576460752303423488 (2^59) is an ERTS bignum and
%% a PBeam inline integer. 2^64 and -(2^64) are bignums on both. The port
%% runs `/usr/bin/true`.
terms() ->
    Port = open_port({spawn_executable, <<"/usr/bin/true">>}, [binary]),
    [id(a), id(true), id(1), id(576460752303423488), id(1.5), id(<<1>>), id(<<>>),
     id([]), id([1]), id([1 | 2]), id({}), id({a, 1}), id(#{}), id(#{a => 1}),
     self(), Port, make_ref(), fun() -> ok end, fun erlang:is_atom/1,
     id(1 bsl 64), id(-(1 bsl 64))].

%% The type-test BIFs of arity 1 that this probe calls through each form.
names() ->
    [is_atom, is_binary, is_bitstring, is_boolean, is_float, is_function, is_list, is_map,
     is_number, is_pid, is_port, is_record, is_reference, is_tuple].

passed(Pairs) -> [Name || {Name, true} <- Pairs].

%% Each call is a `bif` instruction with the fail label 0.
bif() ->
    [passed([{is_atom, is_atom(T)}, {is_binary, is_binary(T)},
             {is_bitstring, is_bitstring(T)}, {is_boolean, is_boolean(T)},
             {is_float, is_float(T)}, {is_function, is_function(T)}, {is_list, is_list(T)},
             {is_map, is_map(T)}, {is_number, is_number(T)}, {is_pid, is_pid(T)},
             {is_port, is_port(T)}, {is_record, erlang:is_record(T)},
             {is_reference, is_reference(T)}, {is_tuple, is_tuple(T)}])
     || T <- terms()].

%% Each call is a `call_fun` of an external fun.
funs() ->
    Funs = [fun erlang:is_atom/1, fun erlang:is_binary/1, fun erlang:is_bitstring/1,
            fun erlang:is_boolean/1, fun erlang:is_float/1, fun erlang:is_function/1,
            fun erlang:is_list/1, fun erlang:is_map/1, fun erlang:is_number/1,
            fun erlang:is_pid/1, fun erlang:is_port/1, fun erlang:is_record/1,
            fun erlang:is_reference/1, fun erlang:is_tuple/1],
    [passed(zip(names(), [F(T) || F <- Funs])) || T <- terms()].

%% Each call is `apply/3`.
applies() ->
    [passed([{N, apply(erlang, N, [T])} || N <- names()]) || T <- terms()].

%% `is_function/2` with an arity that is not a literal is a `bif`
%% instruction with a fail label in a guard. The compiler emits a `test`
%% instruction for an arity-1 type test in a guard, and it does not accept
%% an `is_record/2,3` guard with a name that is not a literal atom.
guard() ->
    [[function_guard(T, id(0)), function_guard(T, id(-1)),
      function_guard(T, id(x))]
     || T <- terms()].

function_guard(T, A) when is_function(T, A) -> yes;
function_guard(_, _) -> no.

%% `is_record/1` in a guard compiles to the `is_any_native_record` test
%% instruction (genop.tab:722-724).
record_guard() ->
    [native_record(T) || T <- terms()].

native_record(T) when is_record(T) -> yes;
native_record(_) -> no.

%% `is_function/2` in a body, through a fun, and through `apply/3`.
arity() ->
    Args = [{fun() -> ok end, 0}, {fun() -> ok end, 1}, {fun erlang:is_atom/1, 1},
            {a, 0}, {fun() -> ok end, -1}, {a, -1}, {a, x}, {a, 1.0},
            {fun() -> ok end, 1 bsl 64}, {a, 1 bsl 64}, {a, -(1 bsl 64)}],
    [[catched(fun() -> is_function(id(F), id(A)) end),
      catched(fun() -> (id(fun erlang:is_function/2))(F, A) end),
      catched(fun() -> apply(erlang, id(is_function), [F, A]) end)]
     || {F, A} <- Args].

%% `is_record/2` and `is_record/3` in a body, through a fun, and through
%% `apply/3`.
record() ->
    Args = [[{a, 1}, a], [{a, 1}, b], [{}, a], [x, a], [{a, 1}, 1],
            [{a, 1}, a, 2], [{a, 1}, a, 3], [{a, 1}, a, x], [{a, 1}, 1, 2],
            [{a, 1}, a, 2.0], [x, a, -1], [{a, 1}, a, 1 bsl 64]]
        %% The bounds of the small integer range of a size, made at run time,
        %% because a literal above 2^31 is a bignum in the literal table.
        ++ [[{a, 1}, a, S] || S <- small_bounds()],
    [[catched(fun() -> body_record(A) end),
      catched(fun() -> apply(id(fun_record(length(A))), A) end),
      catched(fun() -> apply(erlang, id(is_record), A) end)]
     || A <- Args].

%% 2^59, 2^59 - 1, -2^59, and -2^59 - 1.
small_bounds() ->
    Max = id(536870912) * id(1073741824),
    [Max, Max - 1, -Max, -Max - 1].

body_record([T, N]) -> erlang:is_record(id(T), id(N));
body_record([T, N, S]) -> erlang:is_record(id(T), id(N), id(S)).

fun_record(2) -> fun erlang:is_record/2;
fun_record(3) -> fun erlang:is_record/3.

zip([A | As], [B | Bs]) -> [{A, B} | zip(As, Bs)];
zip([], []) -> [].

catched(F) ->
    try F() catch Class:Reason -> {Class, Reason} end.

id(X) -> X.
