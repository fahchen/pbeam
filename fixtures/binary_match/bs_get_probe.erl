-module(bs_get_probe).
-export([split/2, split_units/2, split_bits/2, nibbles/1]).

%% bs_start_match4 no_fail, bs_get_binary2 with the size in a register, and
%% bs_get_tail. A size that is not a non-negative integer, or that is larger
%% than the rest of the binary, fails.
split(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<Head:N/binary, Tail/binary>> -> {Head, Tail};
        _ -> short
    end.

%% bs_get_binary2 with unit 16.
split_units(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<Head:N/binary-unit:16, Tail/binary>> -> {Head, Tail};
        _ -> short
    end.

%% bs_get_binary2 with unit 1: a size that is not a multiple of 8 bits
%% gives a bitstring.
split_bits(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<Head:N/bits, Tail/bits>> -> {Head, Tail};
        _ -> short
    end.

%% bs_start_match4 resume: nibbles/2 gets the match context of nibbles/1.
%% bs_get_tail after bs_set_position, also at a position that is not
%% byte-aligned.
nibbles(<<$a, Rest/binary>>) -> nibbles(Rest, []);
nibbles(_) -> none.

nibbles(<<N:4, Rest/bits>>, Acc) when N =/= 15 -> nibbles(Rest, [N | Acc]);
nibbles(Rest, Acc) -> {Acc, Rest}.
