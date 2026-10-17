-module(binary_module_probe).
-export([results/0, traces/0, shared/0, costs/1, measure/4, id/1]).

%% The value and the error of each binary module BIF of this task. `id/1`
%% keeps an argument from the constant folding of the compiler. The integer
%% literals above the signed 64-bit range are bignum literals.
results() ->
    Bin = <<1, 2, 3, 4, 5>>,
    [at(Bin, 0), at(Bin, 4), at(Bin, 5), at(Bin, -1), at(Bin, 1.0),
     at(Bin, 9223372036854775807), at(Bin, 18446744073709551616), at(<<>>, 0), at(foo, 0),
     at(<<"abc">>, 2),
     part2(Bin, {0, 5}), part2(Bin, {1, 3}), part2(Bin, {5, 0}), part2(Bin, {5, -2}),
     part2(Bin, {2, -2}), part2(Bin, {3, 3}), part2(Bin, {0, 6}), part2(Bin, {0, -6}),
     part2(Bin, {-1, 1}), part2(Bin, {1, 2, 3}), part2(Bin, 1), part2(Bin, {1, 1.0}),
     part3(Bin, 1, 3), part3(Bin, 5, 0), part3(Bin, 5, -2), part3(Bin, 0, 5), part3(Bin, 0, 6),
     part3(Bin, 6, 0), part3(Bin, -1, 1), part3(Bin, 1, -6), part3(Bin, 1, 1.0), part3(Bin, 1, foo),
     part3(foo, 0, 1), part3(Bin, 1, 9223372036854775808), part3(Bin, 9223372036854775808, 1),
     matches(<<"abcabcabd">>, <<"abc">>), matches(<<"abcabcabd">>, <<"abd">>),
     matches(<<"abcabcabd">>, <<"x">>), matches(<<"abcabcabd">>, <<"abcabcabd">>),
     matches(<<"abc">>, <<"abcd">>), matches(<<>>, <<"a">>), matches(<<"aaaa">>, <<"aa">>),
     matches(<<"abcde">>, [<<"bcde">>, <<"bc">>, <<"de">>]), matches(<<"ab.cd.cd">>, <<".">>),
     matches(<<"abc">>, <<>>), matches(<<"abc">>, []), matches(<<"abc">>, [<<>>]),
     matches(<<"abc">>, [<<"a">> | <<"b">>]), matches(<<"abc">>, {bm, <<"a">>}),
     matches(abc, <<"a">>), matches(<<"abc">>, 1),
     copy(<<1, 2>>), copy(<<1, 2>>, 2), copy(<<1, 2>>, 0), copy(<<>>, 3), copy(<<1, 2>>, -1),
     copy(<<1, 2>>, 1.0), copy(foo, 2), copy(<<1, 2>>, 18446744073709551616),
     copy(<<1, 2>>, 4611686018427387904), copy(<<1, 2, 3>>, 9223372036854775808),
     list_to_bin([<<"ab">>, [$c], 3]), list_to_bin([]), list_to_bin([<<>>]), list_to_bin(<<1, 2>>),
     list_to_bin([256]), list_to_bin([-1]), list_to_bin([<<"a">> | <<"b">>]), list_to_bin(foo),
     list_to_bin([[[]]]), list_to_bin([1, [2, [3]]])].

%% The reason, the module, the function, and the arguments of the first
%% stack frame of each error, and the module of its `error_info` location.
%% The pinned ERTS puts the frame `{Module, Function, Args, Location}` of
%% the BIF first (`save_stacktrace`, `beam_common.c:903-1005`). The fixture
%% drops the caller frames, whose locations differ from PBeam.
traces() ->
    [trace(fun() -> binary:at(id(<<1, 2>>), id(5)) end),
     trace(fun() -> binary:part(id(<<1, 2>>), id({0, 3})) end),
     trace(fun() -> binary:part(id(<<1, 2>>), id(0), id(-3)) end),
     trace(fun() -> binary:matches(id(<<"a">>), id(<<>>)) end),
     trace(fun() -> binary:copy(id(<<1>>), id(-1)) end),
     trace(fun() -> binary:list_to_bin(id([256])) end)].

trace(Fun) ->
    try Fun() catch error:Reason:Stack -> describe(Reason, hd(Stack)) end.

describe(Reason, {Module, Function, Args, Location}) ->
    {Reason, {Module, Function, Args}, info_module(Location)}.

info_module([{error_info, #{module := Module}}]) -> Module;
info_module(_) -> none.

%% The source of two parts and of two copies, for the test that a part
%% shares the bytes of its source and that a copy does not.
shared() ->
    Bin = id(<<1, 2, 3, 4, 5>>),
    {Bin, part3(Bin, 1, 3), part2(Bin, {1, 3}), copy(Bin), copy(Bin, 1)}.

at(Bin, Index) ->
    try binary:at(id(Bin), id(Index)) catch Class:Reason -> {Class, Reason} end.

part2(Bin, PosLen) ->
    try binary:part(id(Bin), id(PosLen)) catch Class:Reason -> {Class, Reason} end.

part3(Bin, Position, Length) ->
    try binary:part(id(Bin), id(Position), id(Length)) catch Class:Reason -> {Class, Reason} end.

matches(Subject, Pattern) ->
    try binary:matches(id(Subject), id(Pattern)) catch Class:Reason -> {Class, Reason} end.

copy(Bin) ->
    try binary:copy(id(Bin)) catch Class:Reason -> {Class, Reason} end.

copy(Bin, Count) ->
    try binary:copy(id(Bin), id(Count)) catch Class:Reason -> {Class, Reason} end.

list_to_bin(List) ->
    try binary:list_to_bin(id(List)) catch Class:Reason -> {Class, Reason} end.

%% The summary and the reductions of binary:matches/2 for a subject of N
%% bytes $a followed by Tail. Each measure runs in a new process with the
%% spawn options Opts, after a switch, so that the BIF runs with the budget
%% of a new turn. The base is the cost of the measure with no call.
costs(Opts) ->
    {_, Base} = delta(none, <<>>, <<"a">>, Opts),
    [{N, byte_size(Tail), Pattern, Result, Reds - Base}
     || {N, Tail, Pattern} <- [{10, <<>>, <<"a">>}, {1000, <<>>, <<"a">>},
                               {1000, <<"b">>, <<"b">>}, {100000, <<>>, <<"a">>},
                               {10000, <<>>, <<"aa">>}, {1000, <<>>, [<<"a">>, <<"aa">>]},
                               {1000, <<>>, <<"b">>}, {10, <<>>, <<"aaaaaaaaaa">>},
                               {1000, <<"b">>, <<"aa">>}, {100000, <<"b">>, [<<"a">>, <<"b">>]}],
        {Result, Reds} <- [delta(N, Tail, Pattern, Opts)]].

delta(N, Tail, Pattern, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), N, Tail, Pattern], Opts),
    receive {Pid, Delta} -> Delta end.

measure(Collector, N, Tail, Pattern) ->
    Subject = <<(repeat(<<"a">>, count(N)))/binary, Tail/binary>>,
    erlang:yield(),
    {reductions, Before} = process_info(self(), reductions),
    Result = run(N, Subject, Pattern),
    {reductions, After} = process_info(self(), reductions),
    Collector ! {self(), {Result, After - Before}}.

count(none) -> 0;
count(N) -> N.

run(none, _, _) -> none;
run(_, Subject, Pattern) -> summary(binary:matches(Subject, Pattern)).

%% A summary of a match list, so that a large list stays short in the
%% expected text: the number of the matches, the first match, and the sum of
%% their lengths.
summary(Matches) ->
    {length(Matches), first(Matches), sum(Matches)}.

first([]) -> none;
first([Match | _]) -> Match.

sum([]) -> 0;
sum([{_, Length} | Rest]) -> Length + sum(Rest).

%% Return the argument, so that the compiler does not fold a constant.
id(Term) -> Term.

%% N copies of Bin, built by doubling.
repeat(_, 0) -> <<>>;
repeat(Bin, 1) -> Bin;
repeat(Bin, N) ->
    Half = repeat(Bin, N div 2),
    case N rem 2 of
        0 -> <<Half/binary, Half/binary>>;
        1 -> <<Half/binary, Half/binary, Bin/binary>>
    end.
