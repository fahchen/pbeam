-module(bs_float_probe).
-export([big/2, little/2, native/2, units/2, half/1, unaligned/1, parse/0]).

%% bs_get_float2 with the size in a register: big endian, little endian,
%% and native endian. A size of 0 gives 0.0 and reads no bits. A size that
%% is not 16, 32, or 64, a size that is not a non-negative integer, or a
%% value that is not a finite float fails.
big(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<F:N/float, Rest/binary>> -> {F, Rest};
        _ -> short
    end.

little(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<F:N/float-little, Rest/binary>> -> {F, Rest};
        _ -> short
    end.

native(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<F:N/float-native, Rest/binary>> -> {F, Rest};
        _ -> short
    end.

%% bs_get_float2 with unit 16: the size in bits is N * 16.
units(Bin, N) when is_binary(Bin) ->
    case Bin of
        <<F:N/float-unit:16, Rest/binary>> -> {F, Rest};
        _ -> short
    end.

%% bs_get_float2 with the literal size 16.
half(Bin) when is_binary(Bin) ->
    case Bin of
        <<F:16/float, Rest/binary>> -> {F, Rest};
        _ -> short
    end.

%% bs_get_float2 at a position that is not byte-aligned.
unaligned(Bin) when is_binary(Bin) ->
    case Bin of
        <<_:4, F:32/float, _:4>> -> F;
        _ -> short
    end.

%% erl_parse:parse_term/1 of the tokens of
%% `{short, <<1, 2:16, "xy", 1.5/float>>}.`, which calls
%% eval_bits:expr_grp/3 (erl_parse.yrl:1960-1966). The tokens are those of
%% erl_scan:string/1, so the call needs no erl_scan and no erl_features.
parse() ->
    erl_parse:parse_term(
      [{'{', 1}, {atom, 1, short}, {',', 1}, {'<<', 1}, {integer, 1, 1},
       {',', 1}, {integer, 1, 2}, {':', 1}, {integer, 1, 16}, {',', 1},
       {string, 1, "xy"}, {',', 1}, {float, 1, 1.5}, {'/', 1},
       {atom, 1, float}, {'>>', 1}, {'}', 1}, {dot, 1}]).
