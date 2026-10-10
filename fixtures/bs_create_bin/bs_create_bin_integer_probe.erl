-module(bs_create_bin_integer_probe).
-export([byte/1, big16/1, little16/1, native16/1, wide/1, wide_little/1, nibbles/2, little12/2, odd/2, sized/2, bytes/2, little_bytes/2, zero/1, pair/2, little_pair/2, before_binary/2, append/2, code/1, code_tail/1, code_append/2, sized_code/3, nibble/1, guard/1, code_guard/1, catch_call/2, cost/2]).

%% An integer segment of 8 bits, as `<<X>>`.
byte(X) -> <<X>>.

%% Integer segments of 16 bits in each byte order.
big16(X) -> <<X:16>>.
little16(X) -> <<X:16/little>>.
native16(X) -> <<X:16/native>>.

%% Integer segments of more than 64 bits.
wide(X) -> <<X:72>>.
wide_little(X) -> <<X:72/little>>.

%% Integer segments that do not start on a byte boundary.
nibbles(A, B) -> <<A:4, B:4>>.
little12(A, B) -> <<A:12/little, B:4>>.
odd(A, B) -> <<A:3, B:13/little>>.

%% Integer segments with a size in a register, of unit 1 and unit 8.
sized(X, N) -> <<X:N>>.
bytes(X, N) -> <<X:N/unit:8>>.
little_bytes(X, N) -> <<X:N/little-unit:8>>.

%% An integer segment of size 0.
zero(X) -> <<X:0, "a">>.

%% Two integer segments in one accumulator, in each byte order.
pair(A, B) -> <<A:8, B:8>>.
little_pair(A, B) -> <<A:8/little, B:8/little>>.

%% An integer segment before a binary segment.
before_binary(X, B) -> <<X:8, B/binary>>.

%% An append segment and an integer segment.
append(A, X) -> <<A/binary, X:8>>.

%% utf8 segments.
code(C) -> <<C/utf8>>.
code_tail(C) -> <<C/utf8, "!">>.
code_append(A, C) -> <<A/binary, C/utf8>>.

%% An integer segment with a size in a register before a utf8 segment.
sized_code(X, N, C) -> <<X:N, C/utf8>>.

%% A bitstring that is not a binary.
nibble(X) -> <<X:4>>.

%% bs_create_bin in a guard has a fail label.
guard(X) when <<X:8>> =:= <<1>> -> one;
guard(_) -> other.

code_guard(C) when <<C/utf8>> =:= <<"é"/utf8>> -> e;
code_guard(_) -> other.

%% The exception of a call.
catch_call(F, Args) ->
    try apply(?MODULE, F, Args)
    catch Class:Reason:Stack -> {Class, Reason, hd(Stack)}
    end.

%% The reductions of one bs_create_bin with an integer segment of N bits.
cost(X, N) ->
    R0 = element(2, process_info(self(), reductions)),
    B = <<X:N>>,
    R1 = element(2, process_info(self(), reductions)),
    {R1 - R0, byte_size(B)}.
