-module(error_handler_load_probe).
-export([main/0]).

%% Call a module that the run does not load. ERTS goes to the export entry of
%% the function, which calls `error_handler:undefined_function/3`
%% (`beam_common.c:1294-1342`). That function loads the module through
%% `code:ensure_loaded/1` or `init:ensure_loaded/1`
%% (`error_handler.erl:85-98`, `182-198`) and then runs the function
%% (`error_handler.erl:88-90`). `pbeam_boot` answers `{ensure_loaded, Module}`
%% for the init process (`init.erl:730-750`).
main() ->
    Result = error_handler_load_target:value(a),
    io:format("~p~n", [Result]),
    halt().
