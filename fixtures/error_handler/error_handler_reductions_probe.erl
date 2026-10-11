-module(error_handler_reductions_probe).
-export([main/0]).

%% Print the reductions of the error handler path of
%% `error_handler_probe:reductions/0`. A boot run of `pbeam_boot` has the
%% live code server of `--root`, as ERTS does, so the two runs take the same
%% `code:ensure_loaded/1` path of `error_handler:ensure_loaded/1` and the
%% numbers compare (`error_handler.erl:182-198`, `code.erl:581-583`).
main() ->
    io:format("~w~n", [error_handler_probe:reductions()]),
    halt().
