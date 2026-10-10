-module(bs_match_probe).
-export([unsigned/1, flagged/1, wide/1, prefix/1, skip/1, binaries/1, bits/1,
         tail/1, bit_tail/1]).

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
