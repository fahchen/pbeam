-module(bitstr_probe).
-export([kinds/0, kind/1, bits/0, sizes/0, order/0, print/0, show/1, id/1]).

%% Run the is_bitstr test on one term of each kind that PBeam has. A guard
%% is_bitstring/1 compiles to is_bitstr (opcode 129), as the consolidated
%% String.Chars.impl_for/1 of Elixir 1.20.4 does. The binaries are a
%% literal, the empty literal, and a binary that bs_create_bin makes at run
%% time. Each other term jumps to the fail label.
kinds() ->
    Made = <<"a", (?MODULE:kind(<<"b">>))/binary>>,
    Port = open_port({spawn_executable, "/bin/sh"}, [binary, {args, ["-c", "cat"]}]),
    Terms = [<<"b">>, <<>>, Made, 1, 1 bsl 60, a, "s", {t}, [], [a | b],
             #{k => v}, self(), make_ref(), fun kinds/0, Port],
    Kinds = [?MODULE:kind(Term) || Term <- Terms],
    port_close(Port),
    Kinds.

kind(Term) when is_bitstring(Term) -> Term;
kind(_) -> other.

%% A bitstring whose size is not a multiple of 8. It is a literal, because
%% PBeam runs no bs_create_bin segment that makes one.
bits() -> ?MODULE:kind(<<1:3>>).

%% The bit size and the byte size of bitstring literals.
sizes() ->
    [{bit_size(B), byte_size(B), is_bitstring(B), is_binary(B)}
     || B <- id([<<1:3>>, <<1, 2:4>>, <<255, 7:3>>, <<0:1>>, <<1:8>>])].

%% The term order of bitstrings and binaries.
order() ->
    Terms = id([<<1, 2:4>>, <<1>>, <<1:3>>, <<0:1>>, <<1, 2>>, <<>>, <<1, 3:4>>,
                <<1, 2:5>>, <<32:6>>, <<1:1>>]),
    {lists:sort(Terms),
     [{A < B, A == B, A =:= B} || A <- Terms, B <- id([<<1>>, <<1:3>>, <<1, 2:4>>, <<32>>])]}.

%% Exit with bitstring literals, so that the exit reason prints them.
print() -> exit(id({<<1:3>>, <<1, 2:4>>, <<"ab", 1:1>>})).

%% Exit with the result of a probe function, so that the exit reason
%% prints it.
show(Name) -> exit(?MODULE:Name()).

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.
