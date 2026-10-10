-module(bs_create_bin_append_probe).
-export([two/2, tail/1, built/1, again/1, unit16/2, bits/2, guard/2, catch_two/2, catch_unit16/2, cost/2, cost_again/2]).

%% A <> B in Elixir: an append segment and a binary segment.
two(A, B) -> <<A/binary, B/binary>>.

%% An append segment and a string segment.
tail(A) -> <<A/binary, "!">>.

%% An append to a binary that the same function built.
built(X) ->
    B = <<"a", X/binary>>,
    <<B/binary, "z">>.

%% An append to a binary that an append built.
again(X) ->
    B = <<X/binary, "y">>,
    <<B/binary, "z">>.

%% An append segment of unit 16.
unit16(A, B) -> <<A/binary-unit:16, B/binary>>.

%% An append segment of unit 1.
bits(A, B) -> <<A/bitstring, B/binary>>.

%% An append segment in a guard has a fail label.
guard(A, B) when <<A/binary, B/binary>> =:= <<"ab">> -> ab;
guard(_, _) -> other.

%% The exceptions of an append segment.
catch_two(A, B) ->
    try two(A, B)
    catch Class:Reason:Stack -> {Class, Reason, Stack}
    end.

catch_unit16(A, B) ->
    try unit16(A, B)
    catch Class:Reason:Stack -> {Class, Reason, Stack}
    end.

%% The reductions of one append.
cost(A, B) ->
    R0 = element(2, process_info(self(), reductions)),
    C = <<A/binary, B/binary>>,
    R1 = element(2, process_info(self(), reductions)),
    {R1 - R0, byte_size(C)}.

%% The reductions of an append to a binary that an append built.
cost_again(A, B) ->
    C = <<A/binary, "y">>,
    R0 = element(2, process_info(self(), reductions)),
    D = <<C/binary, B/binary>>,
    R1 = element(2, process_info(self(), reductions)),
    {R1 - R0, byte_size(D)}.
