-module(bit_size_probe).
-export([main/0, body/1, guard/1, rest/1, id/1]).

%% erlang:bit_size/1, as `io_lib_pretty:print_length_binary/7` runs it.
%% The compiler emits each `bit_size/1` call as `gc_bif`: with the fail
%% label 0 in a body, and with a fail label in a guard. The fun and the
%% `apply/3` call run the BIF through its export.
main() ->
    [bit_size(id(<<>>)), bit_size(id(<<"a">>)), bit_size(id(<<"héllo"/utf8>>)),
     bit_size(id(<<"The quick brown fox jumps over the lazy dog">>)),
     (id(fun erlang:bit_size/1))(id(<<"abc">>)), apply(erlang, bit_size, id([<<"ab">>])),
     guard(id(<<>>)), guard(id(<<"abc">>)), guard(id(foo)), guard(id("abc")),
     body(foo), body("abc"), body(1), body({<<"a">>}),
     try apply(erlang, bit_size, id([[1]])) catch Class:Reason -> {Class, Reason} end,
     rest(id(<<1, 2, 3>>)), rest(id(<<1>>)), rest(id(<<>>))].

body(Term) ->
    try bit_size(Term) catch Class:Reason -> {Class, Reason} end.

guard(Term) when bit_size(Term) > 16 -> big;
guard(Term) when bit_size(Term) >= 0 -> small;
guard(_) -> other.

%% The bits after the first byte, read from the match context.
rest(<<_, Rest/bits>>) when bit_size(Rest) >= 8 -> {long, bit_size(Rest)};
rest(<<_, Rest/bits>>) -> {short, bit_size(Rest)};
rest(_) -> none.

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.
