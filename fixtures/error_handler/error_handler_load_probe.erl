-module(error_handler_load_probe).
-export([main/0, lambda/0]).

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

%% Call a decoded fun of a module that no run has. ERTS calls
%% `error_handler:undefined_lambda/3` (`beam_common.c:1708-1728`), which
%% cannot load the module and raises undef with the frame `{Fun, Args, []}`
%% (`error_handler.erl:117-127`, `162-178`).
lambda() ->
    F = binary_to_term(<<131, 112, 0, 0, 0, 0, 1, 7:128, 0, 0, 0, 3, 0, 0, 0, 0,
                         119, 7, "no_such", 97, 2, 98, 255, 255, 255, 255,
                         88, 119, 13, "nonode@nohost", 0:96>>),
    Result = try F(1) catch C:R:S -> {C, R, hd(S)} end,
    io:format("~p~n", [Result]),
    halt().
