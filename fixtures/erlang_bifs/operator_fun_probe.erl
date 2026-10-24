-module(operator_fun_probe).
-export([main/0, limit/0, tail/3]).

%% The operator BIFs and the BIFs that the compiler changes to an
%% instruction, through `apply/3`, a fun call, and a tail call of `apply/3`
%% (`apply_last`), with a module atom that the compiler cannot see. ERTS runs
%% `ebif_bang_2` (`bif.c:2360-2363`), `splus_1` (`erl_arith.c:93-100`),
%% `sle_2` and `sgt_2` (`bif.tab:295`, `298`), `append_2` and `subtract_2`
%% (`erl_bif_lists.c:350-353`, `1103-1105`), `append_element_2`
%% (`bif.c:3083-3108`), and `binary_part_2` (`erl_bif_guard.c:460-473`).
%% Each error gives the class, the reason, and the top frame of the stack
%% trace: the module, the name, whether its arguments are the call arguments,
%% and whether it has `error_info`. A long improper list makes `append/2`
%% and `subtract/2` trap first; the frame keeps the name and the arguments.
main() ->
    Erlang = binary_to_atom(id(<<"erlang">>)),
    Proper = lists:seq(1, 300000),
    Long = Proper ++ id(x),
    Cases = id([{'!', [self(), m]}, {'!', [x, m]}, {'!', [1, m]},
                {'+', [1]}, {'+', [1.5]}, {'+', [1 bsl 70]}, {'+', [a]},
                {'=<', [1, 1.0]}, {'=<', [b, a]}, {'>', [b, a]}, {'>', [1, 1.0]},
                {append, [[1], [2]]}, {append, [a, [2]]}, {append, [[1 | x], [2]]},
                {subtract, [[1, 2, 1], [1]]}, {subtract, [a, [1]]}, {subtract, [[1], x]},
                {append_element, [{a}, b]}, {append_element, [{}, b]}, {append_element, [x, b]},
                {binary_part, [<<1, 2, 3>>, {1, 1}]}, {binary_part, [<<1, 2, 3>>, {3, -2}]},
                {binary_part, [<<1, 2, 3>>, {1, 5}]}, {binary_part, [<<1, 2, 3>>, {1, 1, 1}]},
                {binary_part, [<<1, 2, 3>>, x]}, {binary_part, [x, {0, 0}]},
                {append, [Long, [2]]}, {subtract, [Long, [2]]}, {subtract, [Proper, [a | b]]}]),
    Applied = [result(fun() -> apply(Erlang, F, A) end, A) || {F, A} <- Cases],
    Called = [result(fun() -> call(erlang:make_fun(Erlang, F, length(A)), A) end, A) || {F, A} <- Cases],
    Tail = [result(fun() -> tail(Erlang, F, A) end, A) || {F, A} <- Cases],
    [Applied, Called, Tail, flush()].

%% `append_element/2` of a tuple of `ERTS_MAX_TUPLE_SIZE` elements raises
%% `badarg` (`bif.c:3097-3098`).
limit() ->
    Erlang = binary_to_atom(id(<<"erlang">>)),
    Full = erlang:make_tuple(id(16777215), a),
    result(fun() -> apply(Erlang, append_element, [Full, b]) end, [Full, b]).

%% A tail call of `apply/3`, which the compiler emits as `apply_last`.
tail(Module, Name, Arguments) -> apply(Module, Name, Arguments).

call(Fun, [X]) -> Fun(X);
call(Fun, [X, Y]) -> Fun(X, Y).

result(Fun, Expected) ->
    try norm(Fun())
    catch Class:Reason:Stack ->
        [{Module, Name, Arguments, Info} | _] = Stack,
        {Class, Reason, Module, atom_to_list(Name), Arguments =:= Expected,
         lists:keymember(error_info, 1, Info)}
    end.

norm(Binary) when is_binary(Binary) -> {binary, binary_to_list(Binary)};
norm(Float) when is_float(Float) -> {float, trunc(Float * 10)};
norm(Term) -> Term.

flush() ->
    receive Message -> [Message | flush()] after 0 -> [] end.

id(Term) -> Term.
