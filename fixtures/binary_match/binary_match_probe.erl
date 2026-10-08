-module(binary_match_probe).
-export([kinds/0, kind/1, upper/0, first/1]).

%% Run the bs_start_match3 test on one term of each kind that PBeam has,
%% except a binary. The test fails to its label for each term, as in
%% unicode_util:cp/1 for a list.
kinds() ->
    Terms = [1, 1 bsl 60, a, "s", {t}, [], [a | b], #{k => v}, self(), make_ref(),
             fun kinds/0],
    [?MODULE:kind(Term) || Term <- Terms].

kind(<<_/utf8, _/binary>>) -> utf8;
kind(_) -> other.

%% string:uppercase/1 of a list, as logger_formatter does
%% (logger_formatter.erl:616). unicode_util:cp/1 runs bs_start_match3 on a list.
upper() ->
    [string:uppercase(S) || S <- ["error", "notice", "r"]].

%% bs_start_match3 on a binary.
first(<<C/utf8, _/binary>>) -> {utf8, C};
first(Other) -> {other, Other}.
