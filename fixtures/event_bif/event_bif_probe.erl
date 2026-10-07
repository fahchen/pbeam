-module(event_bif_probe).
-export([to_list/0, size/0, remainder/0, minimum/0, refs/0, range/0, alive/0, queue_len/0,
         term/0, id/1, done/0, wait/0]).

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

%% erlang:rem/2, as logger_formatter:timestamp_to_datetimemicro/2
%% (logger_formatter.erl:572) runs it.
remainder() ->
    [id(A) rem id(B) || {A, B} <- [{7, 3}, {-7, 3}, {7, -3}, {-7, -3}, {6, 3}, {5, 1000000},
                                   {id(576460752303423495), id(576460752303423488)},
                                   {id(-9223372036854775808), -1},
                                   {id(-576460752303423489), id(576460752303423488)}]]
        ++ [try id(A) rem id(B) catch error:R -> {error, R} end
            || {A, B} <- [{1, 0}, {a, 0}, {a, 1}, {1, a}, {1, []}]].

%% erlang:min/2, as io_lib_format:term_bin/8 (io_lib_format.erl:574) runs it.
minimum() ->
    Ref = make_ref(),
    [min(id(1), id(2)), min(id(2), id(1)), min(id(1), id(1)), min(id(1), id(a)), min(id(a), id(1)),
     min(id(a), id(b)), min(id({1}), id({2})), min(id({2, 0}), id({1})), min(id(<<"a">>), id(<<"b">>)),
     min(id(Ref), id(Ref)) =:= Ref, min(id(a), self()), min(id({}), id([]))].

%% erlang:min/2 of two different references.
refs() ->
    min(make_ref(), make_ref()).

%% io:printable_range/0, as io_lib:printable_list/1 (io_lib.erl:1374) runs it.
range() ->
    io:printable_range().

%% erlang:is_process_alive/1, as logger_h_common:log/2 (logger_h_common.erl:181) runs it.
alive() ->
    Dead = spawn(?MODULE, done, []),
    Ref = monitor(process, Dead),
    receive {'DOWN', Ref, process, Dead, _} -> ok end,
    Live = spawn(?MODULE, wait, []),
    Result = [is_process_alive(self()), is_process_alive(Dead), is_process_alive(Live)],
    Live ! stop,
    Result ++ [try is_process_alive(id(T)) catch error:R -> {error, R} end
               || T <- [a, 1, make_ref(), {}, []]].

%% The item message_queue_len of erlang:process_info/2, as logger_olp:flush/2
%% (logger_olp.erl:362) and logger_olp:check_load/1 (logger_olp.erl:483) run it.
queue_len() ->
    Empty = process_info(self(), message_queue_len),
    self() ! a,
    self() ! b,
    Two = process_info(self(), message_queue_len),
    receive a -> ok end,
    receive b -> ok end,
    Live = spawn(?MODULE, wait, []),
    Live ! m1,
    One = process_info(Live, message_queue_len),
    Live ! m2,
    Other = process_info(Live, message_queue_len),
    Live ! stop,
    [Empty, Two, One, Other].

%% persistent_term:get/1, as erl_features:keywords/0 (erl_features.erl:497-499) runs it.
term() ->
    ok = persistent_term:put({?MODULE, key}, value),
    [persistent_term:get({?MODULE, key}),
     try persistent_term:get({?MODULE, missing}) catch error:R -> {error, R} end].

done() -> ok.

wait() ->
    receive stop -> ok end.

id(X) -> X.
