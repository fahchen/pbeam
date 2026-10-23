-module(binary_find_probe).
-export([results/0, compiled/0, scoped/0, costs/1, measure/4]).

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


%% binary:compile_pattern/1 and the compiled patterns of
%% binary:match/2, binary:matches/2, and binary:split/2,3
%% (erl_bif_binary.c:1004-1116, 1244-1271). The reference of a compiled
%% pattern differs between runs, so this probe prints only its tag and
%% whether the second element is a reference.
compiled() ->
    S = <<"xaxbcx">>,
    B = binary:compile_pattern(<<"a">>),
    One = binary:compile_pattern([<<"a">>]),
    A = binary:compile_pattern([<<"a">>, <<"bc">>]),
    [tag(B), tag(One), tag(A),
     binary:match(S, B), binary:matches(<<"xaxa">>, B), binary:split(S, B),
     binary:match(S, A), binary:matches(<<"abc">>, A), binary:split(S, A),
     binary:split(S, A, [global]),
     binary:match(binary:copy(<<"a">>, 400000), binary:compile_pattern(<<"b">>)),
     attempt(fun() -> binary:match(S, {ac, element(2, B)}) end),
     attempt(fun() -> binary:match(S, {bm, element(2, A)}) end),
     attempt(fun() -> binary:match(S, {bm, make_ref()}) end),
     attempt(fun() -> binary:match(S, {xx, element(2, B)}) end),
     attempt(fun() -> binary:compile_pattern(<<>>) end),
     attempt(fun() -> binary:compile_pattern([]) end),
     attempt(fun() -> binary:compile_pattern([<<>>]) end),
     attempt(fun() -> binary:compile_pattern([<<"a">>, 1]) end),
     attempt(fun() -> binary:compile_pattern([<<"a">> | b]) end),
     attempt(fun() -> binary:compile_pattern(abc) end)].

tag(Pattern) ->
    {element(1, Pattern), is_reference(element(2, Pattern))}.

attempt(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

%% binary:match/3 and binary:matches/3 with each option form, and
%% binary:first/1 with each subject form.
scoped() ->
    S = <<"abcabc">>,
    Options = [[], [{scope, {3, 3}}], [{scope, {6, -3}}], [{scope, {0, 1}} | foo], foo,
               [global], [{scope, {0, 7}}], [{scope, {1, 2}}, {scope, {4, 2}}],
               [{scope, {0, -1}}], [{scope, {6, 0}}], [{scope, 1}], [{scope, {a, 1}}],
               [{scope, {0, 1}, x}], [{other, {0, 1}}]],
    [attempt(fun() -> binary:match(S, <<"c">>, O) end) || O <- Options]
        ++ [attempt(fun() -> binary:matches(S, [<<"b">>, <<"c">>], O) end) || O <- Options]
        ++ [attempt(fun() -> binary:match(<<>>, <<"a">>, []) end),
            attempt(fun() -> binary:matches(<<1:3>>, <<"a">>, []) end),
            attempt(fun() -> binary:match(abc, <<"a">>, []) end),
            attempt(fun() -> binary:match(S, <<>>, []) end)]
        ++ [attempt(fun() -> binary:first(B) end) || B <- [<<>>, <<7, 8>>, <<1:9>>, <<1:3>>, a]].
