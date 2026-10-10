-module(iolist_split_probe).
-export([main/0, id/1]).

%% The results of erlang:iolist_to_binary/1 and binary:split/2,3, as the
%% Terra Counter example calls them through Enum.join/2, Inspect.Algebra,
%% and String.split/3. Each call is a body call_ext, and each error gives
%% {error, Reason}. The long inputs trap, so the continuation must give the
%% same result.
main() ->
    [{iolist_to_binary, [to_binary(T) || T <- iodata()]},
     {iolist_to_binary_long, long_binaries()},
     {split2, [split(S, P) || {S, P} <- split2()]},
     {split3, [split(S, P, O) || {S, P, O} <- split3()]},
     {split_long, long_splits()}].

%% Return the argument, so that the compiler does not fold a constant.
id(X) -> X.

to_binary(T) ->
    try iolist_to_binary(id(T)) catch error:R -> {error, R} end.

split(S, P) ->
    try binary:split(id(S), id(P)) catch error:R -> {error, R} end.

split(S, P, O) ->
    try binary:split(id(S), id(P), id(O)) catch error:R -> {error, R} end.

%% Each iodata shape: a binary, [], lists of one binary, bytes, nested
%% lists, binary tails, and terms that are not iodata.
iodata() ->
    [<<1, 2, 3>>, <<>>, [], [<<>>], [<<"abc">>], [1, 2, <<3>>], [[1, [2]], <<3>> | <<4, 5>>],
     [1 | <<2>>], [[], [[]], <<>>], [<<"ab">>, [<<"cd">>, $e], "fg"], [0, 255],
     [1 | 2], [256], [-1], a, [a], [<<1>> | a], {1}, 1, [[1, 2] | b]].

%% Long iodata whose walk traps. Each result is the iovec of the same
%% iodata, which merges each byte and each binary of at most 512 bytes into
%% one binary, so the comparison checks each byte.
long_binaries() ->
    [[iolist_to_binary(id(L))] =:= erlang:iolist_to_iovec(L)
     || L <- [copies(100000, 7), copies(2000, <<"0123456789">>),
              copies(1995, copies(5, <<"0123456789012345678901234567890123456789"
                                     "0123456789012345678901234567890123456789"
                                     "01234567890123456789">>)),
              copies(15983, 1) ++ <<"tail">>]].

split2() ->
    [{<<"a,b,c">>, <<",">>}, {<<"abc">>, <<",">>}, {<<>>, <<",">>}, {<<",">>, <<",">>},
     {<<"abc">>, [<<"c">>]}, {<<"abcabdabcab">>, <<"abcab">>}, {<<"bbab">>, <<"ab">>},
     {<<"xxabxyab">>, [<<"ab">>, <<"xy">>]}, {<<"banana">>, [<<"an">>, <<"nan">>, <<"a">>]},
     {<<"abcd">>, [<<"bc">>, <<"abcd">>]}, {<<"abcd">>, [<<"cd">>, <<"b">>, <<"bcd">>]},
     {a, <<",">>}, {"a,b", <<",">>}, {<<"a">>, <<>>}, {<<"a">>, []}, {<<"a">>, [<<>>]},
     {<<"a">>, [<<"a">> | x]}, {<<"a">>, [a]}, {<<"a">>, a}, {<<"a">>, {bm, make_ref()}},
     {<<"a">>, {ac, <<"a">>}}].

split3() ->
    Text = <<",a,b,,c,,">>,
    [{Text, <<",">>, []}, {Text, <<",">>, [global]}, {Text, <<",">>, [trim]},
     {Text, <<",">>, [global, trim]}, {Text, <<",">>, [trim_all]},
     {Text, <<",">>, [global, trim_all]}, {Text, <<",">>, [global, trim, trim_all]},
     {<<"a,b">>, <<",">>, [trim]}, {<<"ab,">>, <<",">>, [trim]}, {<<",">>, <<",">>, [trim]},
     {<<",ab">>, <<",">>, [trim_all]}, {<<",,">>, <<",">>, [global, trim]},
     {<<>>, <<",">>, [trim]}, {<<>>, <<",">>, [trim_all]}, {<<>>, <<",">>, [global]},
     {<<"banana">>, [<<"a">>], [{scope, {2, 3}}]}, {<<"banana">>, <<"a">>, [global, {scope, {2, 3}}]},
     {<<"abc">>, <<"b">>, [{scope, {3, -2}}]}, {<<"abc">>, <<"b">>, [{scope, {0, 3}}, {scope, {2, 1}}]},
     {<<"abc">>, <<"b">>, [{scope, {3, 0}}]}, {<<"abcabdabcab">>, <<"abcab">>, [global]},
     {<<"aaaa">>, <<"aa">>, [global]}, {<<"bbabbab">>, <<"ab">>, [global]},
     {<<"xyz">>, [<<"ab">>, <<"cd">>], [global]}, {<<"xxabxyabxy">>, [<<"ab">>, <<"xy">>], [global]},
     {<<"banana">>, [<<"an">>, <<"nan">>, <<"a">>], [global]},
     {<<"abcabc">>, [<<"bc">>, <<"abc">>, <<"c">>], [global, trim_all]},
     {<<"a,b,">>, <<",">>, [global | foo]}, {<<"a">>, <<"a">>, foo},
     {<<"a">>, <<"a">>, [bad]}, {<<"a">>, <<"a">>, [{scope, {-1, 1}}]},
     {<<"abc">>, <<"b">>, [{scope, {1, -2}}]}, {<<"abc">>, <<"b">>, [{scope, {4, 0}}]},
     {<<"abc">>, <<"b">>, [{scope, {0, 4}}]}, {<<"abc">>, <<"b">>, [{scope, 1}]},
     {<<"abc">>, <<"b">>, [{scope, {a, 1}}]}, {<<"abc">>, <<"b">>, [{scope, {0, 1, 2}}]},
     {<<"abc">>, <<"b">>, [{other, {0, 1}}]}, {<<"abc">>, <<"b">>, [{scope, {0, id(-576460752303423488) * 16}}]},
     {<<"abc">>, <<>>, [global]}, {<<"abc">>, [], [global]}].

%% Long splits whose search or result build traps. The last split keeps
%% the empty parts before a part that is not empty, also after its result
%% build traps. Each result compares
%% the parts with the expected parts, so the comparison checks each part.
%% The probe builds each subject by doubling and each part list with '++'
%% of halves, so that a PBeam list is not walked cell by cell.
long_splits() ->
    Commas = repeat(50000, <<"ab,">>),
    Plain = repeat(400000, <<"x">>),
    Needle = <<"0123456789abcdefghZ">>,
    Ab = copies(50000, <<"ab">>),
    [binary:split(Commas, <<",">>, [global]) =:= Ab ++ [<<>>],
     binary:split(Commas, <<",">>, [global, trim]) =:= Ab,
     binary:split(Plain, <<"y">>) =:= [Plain],
     binary:split(<<Plain/binary, "y!">>, <<"y">>) =:= [Plain, <<"!">>],
     binary:split(Plain, [<<"ab">>, <<"y">>]) =:= [Plain],
     binary:split(repeat(30000, <<"xyab">>), [<<"ab">>, <<"y">>], [global])
         =:= copies_of(30000, [<<"x">>, <<>>]) ++ [<<>>],
     binary:split(<<Plain/binary, Needle/binary>>, Needle) =:= [Plain, <<>>],
     binary:split(iolist_to_binary([repeat(60000, <<",">>), <<"a,">>]), <<",">>, [global, trim])
         =:= copies(60000, <<>>) ++ [<<"a">>]].

%% A binary of Count copies of Unit, built by doubling.
repeat(0, _) -> <<>>;
repeat(Count, Unit) ->
    Half = repeat(Count div 2, Unit),
    case Count rem 2 of
        0 -> <<Half/binary, Half/binary>>;
        1 -> <<Half/binary, Half/binary, Unit/binary>>
    end.

%% A list of Count copies of Term. Each '++' joins two halves.
copies(Count, Term) -> copies_of(Count, [Term]).

%% A list of the elements of List, Count times.
copies_of(0, _) -> [];
copies_of(1, List) -> List;
copies_of(Count, List) ->
    Half = Count div 2,
    copies_of(Half, List) ++ copies_of(Count - Half, List).
