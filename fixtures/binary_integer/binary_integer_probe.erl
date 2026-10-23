-module(binary_integer_probe).
-export([unsigned/0, common/0, referenced/0, costs/1, measure/3]).

%% binary:encode_unsigned/1,2 and binary:decode_unsigned/1,2 with big and
%% little, zero, bignums, a sub-binary that does not start on a byte
%% boundary, and arguments that are not valid.
unsigned() ->
    Big = 1 bsl 70 + 258,
    <<_:3, Odd:4/binary, _:5>> = id(<<0, 1, 2, 3, 4>>),
    [try F() catch Class:Reason -> {Class, Reason} end
     || F <- [fun() -> binary:encode_unsigned(0) end,
              fun() -> binary:encode_unsigned(258) end,
              fun() -> binary:encode_unsigned(258, little) end,
              fun() -> binary:encode_unsigned(1 bsl 59) end,
              fun() -> binary:encode_unsigned(1 bsl 63, little) end,
              fun() -> binary:encode_unsigned(Big) end,
              fun() -> binary:encode_unsigned(Big, little) end,
              fun() -> binary:encode_unsigned(-1) end,
              fun() -> binary:encode_unsigned(-Big) end,
              fun() -> binary:encode_unsigned(1.0) end,
              fun() -> binary:encode_unsigned(1, native) end,
              fun() -> binary:encode_unsigned(1, "big") end,
              fun() -> binary:decode_unsigned(<<>>) end,
              fun() -> binary:decode_unsigned(<<0, 0>>, little) end,
              fun() -> binary:decode_unsigned(<<0, 1, 2>>) end,
              fun() -> binary:decode_unsigned(<<1, 2, 0>>, little) end,
              fun() -> binary:decode_unsigned(<<0, 255, 255, 255, 255, 255, 255, 255, 255>>) end,
              fun() -> binary:decode_unsigned(binary:encode_unsigned(Big)) end,
              fun() -> binary:decode_unsigned(binary:encode_unsigned(Big, little), little) end,
              fun() -> binary:decode_unsigned(Odd) end,
              fun() -> binary:decode_unsigned(Odd, little) end,
              fun() -> binary:decode_unsigned(<<1:7>>) end,
              fun() -> binary:decode_unsigned(abc) end,
              fun() -> binary:decode_unsigned(<<1>>, native) end,
              fun() -> binary:decode_unsigned(<<1>>, 1) end]].

%% binary:longest_common_prefix/1 and binary:longest_common_suffix/1.
common() ->
    A = binary:copy(id(<<"a">>), 200),
    B = <<(binary:part(A, 0, 150))/binary, "b", (binary:part(A, 0, 49))/binary>>,
    <<_:3, Odd:200/binary, _:5>> = <<0:3, A/binary, 0:5>>,
    Lists = [[<<"abc">>, <<"abd">>], [<<"abc">>], [<<"abc">>, <<>>], [<<"x">>, <<"y">>],
             [<<"abcd">>, <<"abcd">>, <<"ab">>], [A, B], [A, A, B], [A, Odd], [B, Odd],
             [A, binary:part(A, 0, 100)], [<<"ab">>, <<"ab">>, <<"ab">>],
             [], [<<1:7>>], [<<"a">>, <<1:7>>], [<<"a">> | <<"a">>], [<<"a">>, a],
             <<"a">>, [a | b]],
    [{try binary:longest_common_prefix(L) catch C1:R1 -> {C1, R1} end,
      try binary:longest_common_suffix(L) catch C2:R2 -> {C2, R2} end}
     || L <- Lists].

%% binary:referenced_byte_size/1 of heap binaries (at most 64 bytes), of
%% refc binaries, and of sub-binaries of both kinds.
referenced() ->
    Big = binary:copy(id(<<7>>), 200),
    <<P:10/binary, Q:65/binary, _/binary>> = Big,
    <<_:3, X:70/binary, _/bitstring>> = Big,
    <<_:8, Rest/binary>> = Big,
    Small = binary:copy(id(<<7>>), 64),
    <<_:8, SmallRest/binary>> = Small,
    <<_:3, Nine:9/bitstring, _/bitstring>> = Big,
    One = id(1),
    [try binary:referenced_byte_size(T) catch C:R -> {C, R} end
     || T <- [<<>>, <<1, 2, 3>>, Small, SmallRest, Big, Rest, P, Q, X,
              binary:part(Big, 0, 64), binary:part(Big, 1, 65),
              binary:copy(binary:part(Big, 0, 65)), <<One:9>>, <<One:1>>, Nine,
              abc, [1]]].

%% The result and the reductions of binary:longest_common_prefix/1 and
%% binary:longest_common_suffix/1 for two equal binaries of N bytes. Each
%% measure runs in a new process with the spawn options Opts, after a
%% switch, so that the BIF runs with the budget of a new turn. The base is
%% the cost of the measure with no call.
costs(Opts) ->
    {_, Base} = delta(none, prefix, Opts),
    [{N, Direction, Result, Reds - Base}
     || N <- [1, 64, 65, 1000, 4000, 100000, 1000000], Direction <- [prefix, suffix],
        {Result, Reds} <- [delta(N, Direction, Opts)]].

delta(N, Direction, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), N, Direction], Opts),
    receive {Pid, Delta} -> Delta end.

measure(Collector, N, Direction) ->
    L = case N of
            none -> [];
            _ -> [binary:copy(<<"a">>, N), binary:copy(<<"a">>, N)]
        end,
    erlang:yield(),
    {reductions, Before} = process_info(self(), reductions),
    Result = run(Direction, L),
    {reductions, After} = process_info(self(), reductions),
    Collector ! {self(), {Result, After - Before}}.

run(_, []) -> none;
run(prefix, L) -> binary:longest_common_prefix(L);
run(suffix, L) -> binary:longest_common_suffix(L).

id(X) -> X.
