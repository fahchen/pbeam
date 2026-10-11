-module(code_server_probe).
-export([main/0]).

%% Write the code server results of the run, one term on each line: the
%% code mode with the code:ensure_loaded/1 results, the value of the loaded
%% module, and the code:which/1 results, then the code path
%% (code.erl:581-604, 1782-1789; code_server.erl:312-313). The run gives
%% the code path directory fixtures/code_server, which has
%% code_server_target.beam. The start modules of the run do not have
%% code_server_target.
main() ->
    Mode = code:get_mode(),
    Loaded = [erlang:module_loaded(code_server_target),
              code:ensure_loaded(code_server_target),
              erlang:module_loaded(code_server_target),
              code:ensure_loaded(code_server_target),
              code:ensure_loaded(code_server_missing)],
    Value = case erlang:module_loaded(code_server_target) of
                true -> code_server_target:value();
                false -> not_loaded
            end,
    Which = [code:which(code_server_target), code:which(erlang), code:which(lists),
             code:which(code_server_missing)],
    io:format("~p~n", [[Mode, Loaded, Value, Which]]),
    io:format("~p~n", [code:get_path()]).
