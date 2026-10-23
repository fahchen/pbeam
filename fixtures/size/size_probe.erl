-module(size_probe).
-export([main/0, body/1, guard/1, last/1, id/1]).

%% erlang:size/1 (`size_1`, erl_bif_guard.c:315-335). The compiler emits each
%% `size/1` call as `gc_bif`: with the fail label 0 in a body, and with a fail
%% label in a guard. The fun, `apply/3`, and the tail `apply/3` of `last/1`
%% run the BIF through its export, with a module atom the compiler cannot see.
%% A bitstring gives its size in whole bytes, rounded down.
main() ->
    M = binary_to_atom(id(<<"erlang">>), utf8),
    [size(id({})), size(id({a, b, c})), size(id(<<>>)), size(id(<<"abc">>)),
     size(id(<<1:3>>)), size(id(<<1, 2, 3:4>>)),
     (id(fun erlang:size/1))(id({a})), apply(M, size, id([<<"ab">>])), last(id(<<1, 2, 3:7>>)),
     guard(id({a, b, c})), guard(id(<<1:7>>)), guard(id(foo)), guard(id(#{a => 1})),
     body(foo), body("abc"), body(1), body(#{}), body(id(fun id/1)),
     try apply(M, size, id([[1]])) catch Class:Reason -> {Class, Reason} end].

last(Term) ->
    apply(binary_to_atom(id(<<"erlang">>), utf8), size, [Term]).

body(Term) ->
    try size(Term) catch Class:Reason -> {Class, Reason} end.

guard(Term) when size(Term) > 2 -> big;
guard(Term) when size(Term) >= 0 -> small;
guard(_) -> other.

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.
