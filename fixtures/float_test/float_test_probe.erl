-module(float_test_probe).
-export([kinds/0, kind/1, half/1, halves/0]).

%% Run the is_float test on one term of each kind that PBeam has. PBeam has
%% no float term, so each test fails. lists:concat/1 runs the is_float test
%% on each element in lists:thing_to_list/1 (lists.erl:1283-1286), as
%% logger_olp:reset_restart_flag/1 does (logger_olp.erl:467-468).
kinds() ->
    Terms = [1, 1 bsl 60, a, "s", {t}, [], [a | b], <<"b">>, #{k => v}, self(),
             make_ref(), fun kinds/0],
    [?MODULE:kind(Term) || Term <- Terms] ++
        [lists:concat([logger_std_h, "_", default, "_restarting"])].

kind(Term) when is_float(Term) -> float;
kind(_) -> other.

%% The `/` operator on a term that is not known to be a float. The compiler
%% makes the float-register instructions `fconv`, `fmove`, and `fdiv`, as in
%% `string.beam` and `erl_scan.beam`.
half(X) -> X / 2.

%% Run half/1 on one term of each kind that PBeam has and that is not a
%% number. The `fconv` instruction raises badarith for each one.
halves() ->
    Terms = [a, "s", {t}, [], [a | b], <<"b">>, #{k => v}, self(), make_ref(),
             fun kinds/0],
    [try ?MODULE:half(Term) catch error:Reason -> Reason end || Term <- Terms].
