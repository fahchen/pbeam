%% Without the SSA optimizations, the pinned compiler matches a tail of
%% size `all` with `bs_get_binary2`.
-module(bs_all_probe).
-compile([no_ssa_opt]).
-export([bytes/1, bits/1, run/0, expected/0]).

bytes(B) ->
    case B of
        <<X:4, R/binary>> -> {X, R};
        _ -> short
    end.

bits(B) ->
    case B of
        <<X:3, R/bits>> -> {X, R};
        _ -> short
    end.

run() ->
    [bytes(<<1, 2>>), bytes(<<16#12, 16#30:4>>), bytes(<<>>), bits(<<255, 1:2>>), bits(<<1:2>>)].

% The result of `run\/0` on the pinned ERTS.
expected() ->
    [short, {1, <<32>>}, short, {7, <<125:7>>}, short].
