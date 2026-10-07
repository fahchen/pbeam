-module(getenv_probe).
-export([keys/0, values/0, decomposed/0]).

%% os:getenv/1 with each key form. erl_features:init_specs/0
%% (erl_features.erl:466-472) runs it with a flat string.
keys() ->
    [try os:getenv(id(Key)) catch error:Reason -> {error, Reason} end
     || Key <- ["PBEAM_GETENV_ASCII", ["PBEAM_GETENV_", "ASCII"], [[], "PBEAM_GETENV_ASCII", []],
                "PBEAM_GETENV_MISSING", [], [$P, $B, $E, $A, $M, $_, 233], [256],
                'PBEAM_GETENV_ASCII', <<"PBEAM_GETENV_ASCII">>, ["PBEAM_GETENV_", <<"ASCII">>],
                [$P | <<"BEAM_GETENV_ASCII">>], "PBEAM_GETENV_ASCII=", "=", "PBEAM_GETENV_ASCII" ++ [0],
                [-1], [16#D800], [16#110000], [id(1 bsl 60)], [a], {}, 1]].

%% os:getenv/1 values in the native file name encoding.
values() ->
    [os:getenv(Key) || Key <- ["PBEAM_GETENV_EMPTY", "PBEAM_GETENV_COMPOSED", "PBEAM_GETENV_INVALID",
                               "PBEAM_GETENV_CYRILLIC", "PBEAM_GETENV_EQUALS"]].

%% A UTF-8 value with a combining code point, which ERTS composes on macOS.
decomposed() ->
    os:getenv("PBEAM_GETENV_DECOMPOSED").

id(X) -> X.
