-module(event_bif_probe).
-export([to_list/0, size/0, id/1]).

%% erlang:integer_to_list/1, as io_lib:write1/4 (io_lib.erl:816) runs it.
to_list() ->
    [integer_to_list(I) || I <- [0, 7, -15, 1000000, 576460752303423487, -576460752303423488,
                                 id(9223372036854775807), id(-9223372036854775808)]]
        ++ [try integer_to_list(id(T)) catch error:R -> {error, R} end
            || T <- [a, <<"1">>, "1", {1}]].

%% erlang:iolist_size/1, as io_lib:chars_length/1 (io_lib.erl:1820-1823) runs it.
size() ->
    [iolist_size(id(T)) || T <- [<<>>, [], <<"abc">>, "abc", [1, <<2, 3>> | <<4>>],
                                 [[1, [2]], [], 3], [[[]], <<255>>, 0]]]
        ++ [try iolist_size(id(T)) catch error:R -> {error, R} end
            || T <- [a, 1, [256], [-1], [a], [1 | 2], [[1 | a]], {<<>>}]].

id(X) -> X.
