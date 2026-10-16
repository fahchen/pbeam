-module(bs_utf_probe).
-export([retract16/1, get16/1, get16_little/1, get16_native/1, get32/1, get32_little/1, get32_native/1, skip16/1, skip16_little/1, skip32/1, skip32_little/1, offset16/1, offset32/1, retract32/1]).

%% bs_get_utf16 with the flags 0 (big), 2 (little), and 16 (native). A code
%% unit that is not a surrogate is one character. A high surrogate and a
%% low surrogate are one character. A lone surrogate and a short binary
%% fail.
get16(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf16, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

get16_little(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf16-little, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

get16_native(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf16-native, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

%% bs_get_utf32 with the flags 0, 2, and 16. A surrogate and a value above
%% 16#10FFFF fail.
get32(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf32, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

get32_little(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf32-little, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

get32_native(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf32-native, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

%% bs_skip_utf16 and bs_skip_utf32 with the flags 0 and 2.
skip16(Bin) when is_binary(Bin) ->
    case Bin of
        <<_/utf16, Rest/binary>> -> Rest;
        _ -> short
    end.

skip16_little(Bin) when is_binary(Bin) ->
    case Bin of
        <<_/utf16-little, Rest/binary>> -> Rest;
        _ -> short
    end.

skip32(Bin) when is_binary(Bin) ->
    case Bin of
        <<_/utf32, Rest/binary>> -> Rest;
        _ -> short
    end.

skip32_little(Bin) when is_binary(Bin) ->
    case Bin of
        <<_/utf32-little, Rest/binary>> -> Rest;
        _ -> short
    end.

%% A segment at the bit offset 4.
offset16(Bin) when is_binary(Bin) ->
    case Bin of
        <<_:4, C/utf16, _:4, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

offset32(Bin) when is_binary(Bin) ->
    case Bin of
        <<_:4, C/utf32, _:4, Rest/binary>> -> {C, Rest};
        _ -> short
    end.

%% A bs_get_utf16 that fails keeps the position, so the next clause reads
%% the same 16 bits.
retract16(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf16, Rest/binary>> -> {C, Rest};
        <<X:16, Rest/binary>> -> {int, X, Rest};
        _ -> short
    end.

%% A bs_get_utf32 that fails moves the position back by 32 bits, so the
%% next clause reads the same bits.
retract32(Bin) when is_binary(Bin) ->
    case Bin of
        <<C/utf32, Rest/binary>> -> {C, Rest};
        <<X:32, Rest/binary>> -> {int, X, Rest};
        _ -> short
    end.
