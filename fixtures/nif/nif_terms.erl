%% The module of the NIF library fixtures/nif/nif_terms.c: probe/2 runs the
%% term and memory enif calls of its first argument.
-module(nif_terms).
-export([probes/1, mismatches/1, call/3]).

-nifs([probe/2]).

%% Load the library `Path` and return the result of each probe case.
probes(Path) ->
    ok = erlang:load_nif(Path, 0),
    [try_probe(Kind, Arg) || {Kind, Arg} <- cases()].

%% Load the library `Path` and return each probe case whose result is not
%% the result of the pinned ERTS, as {Case, Result, Expected}.
mismatches(Path) ->
    mismatches(cases(), probes(Path), expected()).

%% Load the library `Path` and return the result of one probe call.
call(Path, Kind, Arg) ->
    ok = erlang:load_nif(Path, 0),
    try_probe(Kind, Arg).

mismatches([_ | Cases], [Result | Results], [Result | Expected]) ->
    mismatches(Cases, Results, Expected);
mismatches([Case | Cases], [Result | Results], [Other | Expected]) ->
    [{Case, Result, Other} | mismatches(Cases, Results, Expected)];
mismatches([], [], []) ->
    [].

cases() ->
    %% The compiler keeps the bitstrings out of the literal table.
    Bytes = id(<<1, 2, 3, 4>>),
    <<_:3, Offset:16/bits, _/bits>> = Bytes,
    [{0, 0},
     {1, 0}, {1, 3}, {1, 100}, {1, -1},
     {2, 0}, {2, 100},
     {3, <<"ab">>}, {3, Offset}, {3, "ab"}, {3, a},
     {4, <<"ab">>}, {4, Offset}, {4, []}, {4, [97, <<"b">>, [99, []] | <<"d">>]}, {4, [<<>>]},
     {4, [256]}, {4, [-1]}, {4, [97 | b]}, {4, [a]}, {4, a},
     {5, [abc | 4]}, {5, [abc | 8]}, {5, [abc | 3]}, {5, [abc | 0]}, {5, ['' | 1]}, {5, ['é' | 2]},
     {5, ['é' | 1]}, {5, ['ā' | 8]}, {5, [1 | 4]},
     {6, [abc | 4]}, {6, [abc | 3]}, {6, ['é' | 3]}, {6, ['é' | 2]}, {6, ['ā' | 3]}, {6, [1 | 4]},
     {7, 1.5}, {7, -0.0}, {7, 1}, {7, a},
     {8, 0}, {8, 1},
     {9, 7}, {9, -1}, {9, 1 bsl 59}, {9, 1 bsl 63 - 1}, {9, -(1 bsl 63)}, {9, 1 bsl 63},
     {9, -(1 bsl 63) - 1}, {9, 1.0}, {9, a},
     {10, [1, 2]}, {10, [a | b]}, {10, []}, {10, a},
     {11, [1, 2, 3]}, {11, []}, {11, [1 | b]}, {11, a},
     {12, 0}, {12, 1}, {12, 2}, {12, 3}, {12, 4},
     {13, 1},
     {14, {my, reason}}, {14, badarg},
     {15, {my, reason}},
     {17, [abc | 8]}].

%% The results of probes/1 on the pinned ERTS (OTP 29.1.1), in the order of
%% cases/0.
expected() ->
    Letters = <<"abcdefghijklmnopqrstuvwxyz">>,
    Bad = {error, badarg},
    X = $x,
    [{<<"abcdefgh">>, true, true},
     {<<>>, true, none}, {<<"abc">>, true, $a},
     {<<Letters/binary, Letters/binary, Letters/binary, "abcdefghijklmnopqrstuv">>, true, $a},
     alloc_failed,
     released, released,
     <<"ab">>, <<8, 16>>, Bad, Bad,
     <<"ab">>, <<8, 16>>, <<>>, <<"abcd">>, <<>>,
     Bad, Bad, Bad, Bad, Bad,
     {4, <<"abc", 0, X, X, X, X>>}, {4, <<"abc", 0, X, X, X, X>>}, {0, <<"xxxxxxxx">>},
     {0, <<"xxxxxxxx">>}, {1, <<0, "xxxxxxx">>}, {2, <<233, 0, "xxxxxx">>},
     {0, <<"xxxxxxxx">>}, {0, <<"xxxxxxxx">>}, {0, <<"xxxxxxxx">>},
     {4, <<"abc", 0, X, X, X, X>>}, {0, <<"xxxxxxxx">>}, {3, <<195, 169, 0, "xxxxx">>},
     {0, <<"xxxxxxxx">>}, {3, <<196, 129, 0, "xxxxx">>}, {0, <<"xxxxxxxx">>},
     1.5, -0.0, Bad, Bad,
     Bad, Bad,
     7, -1, 576460752303423488, 9223372036854775807, -9223372036854775808, Bad,
     Bad, Bad, Bad,
     {1, [2]}, {a, b}, Bad, Bad,
     3, 0, Bad, Bad,
     hello, 'été', list_to_atom(duplicate(255, $a)), Bad, '',
     {[], [1, a, 1], [1 | a], [], [1, a], {}},
     {error, {my, reason}}, {error, badarg},
     {error, {my, reason}},
     {0, <<"xxxxxxxx">>}].

duplicate(0, _) -> [];
duplicate(N, Element) -> [Element | duplicate(N - 1, Element)].

id(Term) -> Term.

try_probe(Kind, Arg) ->
    try probe(Kind, Arg) catch Class:Reason -> {Class, Reason} end.

probe(_Kind, _Arg) -> erlang:nif_error(undef).
