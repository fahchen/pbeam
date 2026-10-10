-module(byte_size_probe).
-export([main/0, bits/0, body/1, guard/1, id/1]).

%% erlang:byte_size/1, as the Elixir `byte_size/1` and `IO.inspect/1` run
%% it. The compiler emits each `byte_size/1` call as `gc_bif`: with the fail
%% label 0 in a body, and with a fail label in a guard. The fun and the
%% `apply/3` call run the BIF through its export.
main() ->
    [byte_size(id(<<>>)), byte_size(id(<<"a">>)), byte_size(id(<<"héllo"/utf8>>)),
     byte_size(id(<<"The quick brown fox jumps over the lazy dog">>)),
     (id(fun erlang:byte_size/1))(id(<<"abc">>)), apply(erlang, byte_size, id([<<"ab">>])),
     guard(id(<<>>)), guard(id(<<"abc">>)), guard(id(foo)), guard(id("abc")),
     body(foo), body("abc"), body(1), body({<<"a">>}),
     try apply(erlang, byte_size, id([[1]])) catch Class:Reason -> {Class, Reason} end].

%% A bitstring whose size is not a multiple of 8 bits.
bits() ->
    [byte_size(id(<<1:3>>)), byte_size(id(<<1, 2, 3, 4:1>>))].

body(Term) ->
    try byte_size(Term) catch Class:Reason -> {Class, Reason} end.

guard(Term) when byte_size(Term) > 2 -> big;
guard(Term) when byte_size(Term) >= 0 -> small;
guard(_) -> other.

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.
