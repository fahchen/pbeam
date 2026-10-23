-module(bs_match_probe).
-export([unsigned/1, flagged/1, wide/1, prefix/1, skip/1, binaries/1, bits/1,
         tail/1, bit_tail/1, unaligned_utf8/1, match_string/1,
         match_string_offset/1, match_string_bits/1, skip_size/2,
         skip_size_bits/2, get_size/2, get_size_flagged/2,
         bitstrings/0, literals/0]).

%% The integer command: unsigned big-endian integers below, at, and above
%% one byte, also at positions that are not byte-aligned, and a size with a
%% unit.
unsigned(<<A:3, B:5, C:12, D:4, E:16, F:2/unit:8, G:1, H:7>>) -> [A, B, C, D, E, F, G, H];
unsigned(_) -> false.

%% The integer command with the flags little, native, and signed.
flagged(<<A:16/little, B:12/little, C:4, D:8/signed, E:12/little-signed, F:4/signed,
          G:32/native, H:24/little-signed>>) ->
    [A, B, C, D, E, F, G, H];
flagged(_) -> false.

%% The integer command with 60 or more bits: ERTS makes a bignum when the
%% value is not a small integer, and reads a segment above 64 bits with
%% erts_bs_get_integer_2.
wide(<<A:64, B:64/signed, C:68/little-signed, D:60, E:59/signed, F:5>>) -> [A, B, C, D, E, F];
wide(_) -> false.

%% The =:= command, also a string above 31 bits that the compiler splits,
%% and the get_tail command. A failed clause continues at the next clause.
prefix(<<"ab", Rest/binary>>) -> {ab, Rest};
prefix(<<"hello world", Rest/binary>>) -> {hello, Rest};
prefix(<<"hello", Rest/binary>>) -> {hi, Rest};
prefix(<<1:4, X:4, _/binary>>) -> {nibble, X};
prefix(_) -> other.

%% The skip command, in a read with other segments and alone.
skip(<<_:16, X, _:3, Y:5, _:104, Z>>) -> [X, Y, Z];
skip(_) -> false.

%% The binary command: byte-aligned, unaligned, and above 64 bits.
binaries(<<A:2/binary, _:4, B:1/binary, _:4, C:9/binary>>) -> [A, B, C];
binaries(_) -> false.

%% The binary command with a size that is not a multiple of 8 bits.
bits(<<A:3/bits, _:5>>) -> A;
bits(_) -> false.

%% The get_tail command.
tail(<<_, Rest/binary>>) -> Rest;
tail(_) -> false.

%% The get_tail command of a tail that is not a multiple of 8 bits.
bit_tail(<<_:4, Rest/bits>>) -> Rest;
bit_tail(_) -> false.

%% The skip command moves the position to a bit that is not byte-aligned,
%% and bs_get_utf8 then reads the character at that position.
unaligned_utf8(<<_:4, C/utf8, _/bits>>) -> C;
unaligned_utf8(_) -> false.

%% The string command of a match: a string of 104 bits, which is above the
%% 64-bit value of a `=:=` command, at the start and at the bit offset 4.
match_string(<<"icrosoft.icon", Rest/binary>>) -> Rest;
match_string(_) -> false.

match_string_offset(<<_:4, "icrosoft.icon", Rest/binary>>) -> Rest;
match_string_offset(_) -> false.

%% The same string at the bit offset 4 without the check that the tail is a
%% whole number of bytes, so a binary whose bits are 4 after a multiple of 8
%% matches.
match_string_bits(<<_:4, "icrosoft.icon", _/bits>>) -> ok;
match_string_bits(_) -> false.

%% The skip command of a match with a variable size of 8-bit units and of
%% bits, which a command list cannot hold.
skip_size(Bin, Size) when is_binary(Bin) ->
    case Bin of
        <<_:Size/unit:8, Rest/binary>> -> Rest;
        _ -> false
    end.

skip_size_bits(Bin, Size) when is_binary(Bin) ->
    case Bin of
        <<_:Size, Rest/binary>> -> Rest;
        _ -> false
    end.

%% The integer command of a match with a variable size, without and with
%% the flags `little` and `signed`.
get_size(Bin, Size) when is_binary(Bin) ->
    case Bin of
        <<X:Size, Rest/binary>> -> {X, Rest};
        _ -> false
    end.

get_size_flagged(Bin, Size) when is_binary(Bin) ->
    case Bin of
        <<X:Size/little-signed, Rest/binary>> -> {X, Rest};
        _ -> false
    end.

%% `bits` and `bitstring` segments, and the unit check of a `binary` tail,
%% on bitstring literals whose size is not a multiple of 8 bits.
bitstrings() ->
    Inputs = [<<1:12>>, <<5:13>>, <<1, 2>>, <<1:3>>, <<>>],
    [{split_bits(B), split_binary_tail(B), split_unit(B)} || B <- Inputs].

split_bits(<<A:3/bits, B:2/bitstring, Rest/bits>>) -> {A, B, Rest};
split_bits(_) -> false.

split_binary_tail(<<A:4/bits, Rest/binary>>) -> {A, Rest};
split_binary_tail(_) -> false.

split_unit(<<A:1/bits, Rest/binary-unit:4>>) -> {A, Rest};
split_unit(_) -> false.

%% The bitstring literals that the tests expect, which the loader reads as
%% `BIT_BINARY_EXT` terms.
literals() ->
    {<<5:3>>, <<5:4>>, <<0:3>>, <<8, 2:5>>, <<242, 3:4>>,
     [{{<<0:3>>, <<0:2>>, <<1:7>>}, {<<0:4>>, <<1>>}, false},
      {{<<0:3>>, <<0:2>>, <<5>>}, false, {<<0:1>>, <<0, 5:4>>}},
      {{<<0:3>>, <<0:2>>, <<32, 2:3>>}, false, false},
      {false, false, false},
      {false, false, false}]}.
