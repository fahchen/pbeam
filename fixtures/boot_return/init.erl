%% A boot module `init` whose `boot/1` returns, so `erl_init:start/2`
%% returns and the first process exits with the reason `normal`
%% (erl_init.erl `run/3`).
-module(init).
-export([boot/1]).

boot(_Arguments) ->
    ok.
