-module(binary_part_probe).
-export([main/0, bits/0, body/3, guard/3, id/1]).

%% erlang:binary_part/3, as Terra.Input.Parser.do_parse/3 runs it. The
%% compiler emits each `binary_part/3` call as `gc_bif3`: with the fail label
%% 0 in a body, and with a fail label in a guard. The fun and the `apply/3`
%% call run the BIF through its export. A negative length takes the bytes
%% before the start. -9223372036854775808 is the smallest length that fits in
%% a signed 64-bit word, and its negation does not.
main() ->
    [binary_part(id(<<"hello">>), id(0), id(5)), binary_part(id(<<"hello">>), id(1), id(3)),
     binary_part(id(<<"hello">>), id(5), id(0)), binary_part(id(<<"hello">>), id(5), id(-2)),
     binary_part(id(<<"hello">>), id(2), id(-2)), binary_part(id(<<>>), id(0), id(0)),
     (id(fun erlang:binary_part/3))(id(<<"abc">>), id(1), id(2)),
     apply(erlang, binary_part, id([<<"abc">>, 2, -2])),
     guard(id(<<"hello">>), id(1), id(3)), guard(id(<<"hello">>), id(0), id(2)),
     guard(id(<<"hello">>), id(4), id(2)), guard(id(foo), id(0), id(0)),
     body(<<"hello">>, 0, 6), body(<<"hello">>, 6, 0), body(<<"hello">>, 1, -2),
     body(<<"hello">>, -1, 1), body(foo, 0, 0), body("abc", 0, 1), body(<<"a">>, a, 0),
     body(<<"a">>, 0, []), body(<<"a">>, 0, -9223372036854775808),
     body(<<"a">>, 9223372036854775807, 1), body(<<"a">>, 0, 9223372036854775807),
     try apply(erlang, binary_part, id([<<"a">>, 1, 1])) catch Class:Reason -> {Class, Reason} end].

%% A bitstring whose size is not a multiple of 8 bits.
bits() ->
    body(id(<<1, 2:4>>), 0, 1).

body(Binary, Start, Length) ->
    try binary_part(id(Binary), id(Start), id(Length)) catch Class:Reason -> {Class, Reason} end.

guard(Binary, Start, Length) when binary_part(Binary, Start, Length) =:= <<"ell">> -> ell;
guard(Binary, Start, Length) when byte_size(binary_part(Binary, Start, Length)) > 0 -> part;
guard(_, _, _) -> other.

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.
