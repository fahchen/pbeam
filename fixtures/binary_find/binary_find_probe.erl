-module(binary_find_probe).
-export([results/0, costs/1, measure/4]).

%% binary:match/2 with each subject and pattern form. A pattern of one
%% binary runs the Boyer-Moore search, and two or more binaries run the
%% Aho-Corasick search.
results() ->
    S = <<"abcabcabd">>,
    [match(Subject, Pattern)
     || {Subject, Pattern} <- [{S, <<"abd">>}, {S, <<"c">>}, {S, <<"x">>}, {S, [<<"bc">>]},
                               {S, S}, {S, <<S/binary, "x">>}, {<<>>, <<"a">>},
                               {<<"aaaa">>, <<"aa">>}, {<<"xxabab">>, <<"abab">>},
                               {<<"abaabab">>, <<"abab">>}, {<<"cabcab">>, <<"abcab">>},
                               {S, [<<"ca">>, <<"bd">>]}, {S, [<<"x">>, <<"y">>]},
                               {<<"abcd">>, [<<"bc">>, <<"abc">>, <<"b">>]},
                               {<<"xabcd">>, [<<"bcd">>, <<"ab">>, <<"abc">>]},
                               {S, <<>>}, {S, []}, {S, [<<>>]}, {S, [<<"a">> | <<"b">>]},
                               {S, [<<"a">>, <<>>]}, {S, [<<"a">>, b]}, {S, {bm, <<"a">>}},
                               {S, {ac, make_ref()}}, {S, {bm}}, {S, {1, 2}}, {S, abc},
                               {S, "a"}, {S, [abc]}, {abc, <<"a">>}, {"abc", <<"a">>},
                               {abc, abc}]].

match(Subject, Pattern) ->
    try binary:match(Subject, Pattern) catch Class:Reason -> {Class, Reason} end.

%% The result and the reductions of binary:match/2 for subjects of N bytes
%% $a followed by Tail, with the sizes of Tail and Needle. Each measure runs in a new process with the
%% spawn options Opts, after a switch, so that the BIF runs with the
%% budget of a new turn. The base is the cost of the measure with no call.
costs(Opts) ->
    {_, Base} = delta(none, <<>>, <<>>, Opts),
    [{N, byte_size(Tail), byte_size(Needle), Result, Reds - Base}
     || {N, Tail, Needle} <- [{10, <<>>, <<"b">>}, {10, <<"b">>, <<"b">>},
                              {100000, <<>>, <<"b">>}, {100000, <<"b">>, <<"b">>},
                              {319905, <<>>, <<"b">>}, {319906, <<>>, <<"b">>},
                              {1000000, <<>>, <<"b">>}, {1000000, <<"b">>, <<"b">>},
                              {10000, <<>>, <<"ba">>}, {10000, <<"ba">>, <<"ba">>},
                              {100000, <<>>, <<"ba">>}, {100000, <<"ba">>, <<"ba">>},
                              {100000, <<"bcdefghija">>, <<"bcdefghija">>},
                              {1000, <<>>, <<"aaaaaaaaaaaaaaaaaaaab">>}],
        {Result, Reds} <- [delta(N, Tail, Needle, Opts)]].

delta(N, Tail, Needle, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), N, Tail, Needle], Opts),
    receive {Pid, Delta} -> Delta end.

measure(Collector, N, Tail, Needle) ->
    Subject = <<(repeat(<<"a">>, count(N)))/binary, Tail/binary>>,
    erlang:yield(),
    {reductions, Before} = process_info(self(), reductions),
    Result = run(N, Subject, Needle),
    {reductions, After} = process_info(self(), reductions),
    Collector ! {self(), {Result, After - Before}}.

count(none) -> 0;
count(N) -> N.

run(none, _, _) -> none;
run(_, Subject, Needle) -> binary:match(Subject, Needle).

%% N copies of Bin, built by doubling.
repeat(_, 0) -> <<>>;
repeat(Bin, 1) -> Bin;
repeat(Bin, N) ->
    Half = repeat(Bin, N div 2),
    case N rem 2 of
        0 -> <<Half/binary, Half/binary>>;
        1 -> <<Half/binary, Half/binary, Bin/binary>>
    end.
