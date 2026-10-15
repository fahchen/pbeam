-module(is_integer_probe).
-export([main/0, big/0, body/3, guard/3, id/1]).

%% erlang:is_integer/3, as inet_db:crypto_rand_range/1 runs it. The compiler
%% emits each `is_integer/3` call as `bif3`: with the fail label 0 in a body,
%% and with a fail label in a guard. The fun and the `apply/3` call run the
%% BIF through its export.
main() ->
    [body(5, 0, 10), body(0, 0, 10), body(10, 0, 10), body(11, 0, 10), body(-1, 0, 10),
     body(foo, 0, 10), body(5, 10, 0), body(5, 5, 5),
     body(9223372036854775807, -9223372036854775808, 9223372036854775807),
     body(-9223372036854775808, -9223372036854775807, 0),
     body(5, a, 10), body(5, 0, b), body(foo, a, 10), body(5, 0, [1]),
     guard(id(5), id(0), id(10)), guard(id(11), id(0), id(10)), guard(id(foo), id(0), id(10)),
     guard(id(5), id(a), id(10)), guard(id(5), id(0), id(b)),
     (id(fun erlang:is_integer/3))(id(3), id(1), id(3)),
     apply(erlang, is_integer, id([4, 1, 3])),
     try apply(erlang, is_integer, id([4, x, 3])) catch Class:Reason -> {Class, Reason} end].

%% A bound outside the 64-bit range is a bignum.
big() ->
    body(5, 0, 1 bsl 70).

body(Term, Low, High) ->
    try is_integer(id(Term), id(Low), id(High)) catch Class:Reason -> {Class, Reason} end.

guard(Term, Low, High) when is_integer(Term, Low, High) -> in;
guard(_, _, _) -> out.

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.
