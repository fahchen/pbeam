-module(iolist_split_probe).
-export([main/0, id/1]).

%% The results of erlang:iolist_to_binary/1, as the Terra Counter example
%% calls it through Enum.join/2 and Inspect.Algebra. Each call is a body
%% call_ext, and each error gives {error, Reason}. The long inputs trap, so
%% the continuation must give the same result.
main() ->
    [{iolist_to_binary, [to_binary(T) || T <- iodata()]},
     {iolist_to_binary_long, long_binaries()}].

%% Return the argument, so that the compiler does not fold a constant.
id(X) -> X.

to_binary(T) ->
    try iolist_to_binary(id(T)) catch error:R -> {error, R} end.

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

%% A list of Count copies of Term. Each '++' joins two halves.
copies(0, _) -> [];
copies(1, Term) -> [Term];
copies(Count, Term) ->
    Half = Count div 2,
    copies(Half, Term) ++ copies(Count - Half, Term).
