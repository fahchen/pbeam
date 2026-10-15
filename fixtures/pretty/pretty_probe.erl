-module(pretty_probe).
-export([main/0]).

%% Representative terms: atoms, integers, a bignum, a float, lists, a
%% string, an improper list, tuples, maps, the empty binary, a nested term,
%% and a list that `~p` breaks over lines. The keys of a map with more
%% than one key are integers: the order of atom keys depends on the atom
%% table of the run.
terms() ->
    [a, 'B c', 1, -7, 1 bsl 64, 1.5, [], [1, 2], "abc", [1 | 2], {}, {a, 1},
     #{}, #{1 => a, 2 => [b]}, <<>>, {a, [b, #{c => {d, e}}], 2.0},
     [{key, N} || N <- lists:seq(1, 12)]].

%% Binaries that are not empty. On PBeam, `~p` of such a binary stops at
%% `erlang:bit_size/1` in `io_lib_pretty:print_length_binary/7`
%% (io_lib_pretty.erl:1116), so only `~w` writes them.
binaries() ->
    [<<1, 2>>, <<"text">>, {a, [b, #{c => {d, <<"e">>}}], 2.0}].

%% The text of `io_lib:format/2` with the directive `~Letter` for each
%% term, one term on each line, as a flat list of characters.
show(Letter, Terms) ->
    Directive = [$~ | atom_to_list(Letter)],
    lists:flatten(lists:join($\n, [io_lib:format(Directive, [T]) || T <- Terms])).

%% Write the `~p` text and the `~w` text of the terms.
main() ->
    io:format("~s~n~s~n", [show(p, terms()), show(w, terms() ++ binaries())]).
