-module(bs_create_bin_utf_probe).
-export([utf16/1, utf16_little/1, utf16_native/1, utf32/1, utf32_little/1,
         utf32_native/1, utf16_between/1, utf32_between/1, utf16_bits/1,
         utf32_bits/1, mixed/2, ordered/2, append_utf16/2, call/2]).

%% The utf16 segment of a code point below 0x10000 and of a code point
%% that needs a surrogate pair, in each byte order.
utf16(X) -> <<X/utf16>>.
utf16_little(X) -> <<X/utf16-little>>.
utf16_native(X) -> <<X/utf16-native>>.

%% The utf32 segment of a code point, in each byte order.
utf32(X) -> <<X/utf32>>.
utf32_little(X) -> <<X/utf32-little>>.
utf32_native(X) -> <<X/utf32-native>>.

%% A utf16 and a utf32 segment between other segments, so that the writer
%% is not at the start of the result.
utf16_between(X) -> <<"a", X/utf16, "b">>.
utf32_between(X) -> <<"a", X/utf32, "b">>.

%% A utf16 and a utf32 segment at a bit offset that is not a multiple of
%% 8, so that the bits of the segment are 4 bits after a segment and 4
%% bits before one.
utf16_bits(X) -> <<0:4, X/utf16, 0:4>>.
utf32_bits(X) -> <<0:4, X/utf32, 0:4>>.

%% A utf16 segment after an integer segment of a size in bits.
mixed(X, Bits) -> <<1:Bits, X/utf16>>.

%% A binary segment of size `all` as the first segment, then a utf16
%% segment. The bits of the utf16 segment go after the bytes of the binary.
append_utf16(A, X) -> <<A/binary, X/utf16>>.

%% A utf16 segment with a value, then an integer segment of `Bits` bits.
%% The size pass checks the integer size after the utf16 segment, and the
%% construct pass then fails for the utf16 value, so the first failure
%% shows which pass owns it.
ordered(X, Bits) -> <<X/utf16, 1:Bits>>.

%% The result of `Function` with `Args`, or the class, the reason, and the
%% first frame of the stack trace of the exception that it raises.
call(Function, Args) ->
    try apply(?MODULE, Function, Args)
    catch Class:Reason:Stack -> {Class, Reason, hd(Stack)}
    end.
