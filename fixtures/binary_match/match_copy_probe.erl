-module(match_copy_probe).
-export([parse/1]).

%% parse/2 gives its match context to decode_codepoint/1, as
%% `Terra.Input.Parser.extend/3` gives it to `decode_codepoint/1`.
parse(Input) when is_binary(Input) -> parse(Input, []).

parse(<<Byte, _/binary>> = Rest, Acc) when Byte >= 16#C2 ->
    case decode_codepoint(Rest) of
        {ok, Codepoint, Tail} -> parse(Tail, [Codepoint | Acc]);
        Other -> lists:reverse([Other | Acc])
    end;
parse(<<Byte, Rest/binary>>, Acc) -> parse(Rest, [Byte | Acc]);
parse(<<>>, Acc) -> lists:reverse(Acc).

%% The form of `Terra.Input.Parser.decode_codepoint/1`: bs_start_match4
%% resume, move of the match context to a Y register, bs_set_position,
%% byte_size, and bs_get_tail on the copy in the Y register, then
%% bs_start_match4 no_fail on a binary with another destination.
decode_codepoint(<<Byte, _/binary>> = Binary) ->
    case utf8_length(Byte) of
        invalid ->
            invalid;
        Length when byte_size(Binary) < Length ->
            incomplete;
        Length ->
            Candidate = binary_part(Binary, 0, Length),
            Tail = binary_part(Binary, Length, byte_size(Binary) - Length),
            case Candidate of
                <<Codepoint/utf8>> -> {ok, Codepoint, Tail};
                _ -> invalid
            end
    end.

utf8_length(Byte) when Byte >= 16#C2, Byte =< 16#DF -> 2;
utf8_length(Byte) when Byte >= 16#E0, Byte =< 16#EF -> 3;
utf8_length(Byte) when Byte >= 16#F0, Byte =< 16#F4 -> 4;
utf8_length(_) -> invalid.
