-module(bitstr_probe).
-export([kinds/0, kind/1, bits/0]).

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
