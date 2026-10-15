-module(on_load_server_probe).
-export([main/0, failure/0]).

%% Write the results of code_server for modules with an -on_load function
%% whose function returns ok, one term on each line. code:ensure_loaded/1
%% loads on_load_ok, and a call loads on_load_call on demand through
%% error_handler (error_handler.erl:85-98; code.erl:581-600). code_server
%% runs the on_load function in a new process and makes the code current
%% through erts_code_purger (code_server.erl:1348-1365, 1386-1403;
%% erts_code_purger.erl:166-170).
main() ->
    Ensured = [erlang:module_loaded(on_load_ok), code:ensure_loaded(on_load_ok),
               erlang:module_loaded(on_load_ok), on_load_ok:value(),
               code:ensure_loaded(on_load_ok)],
    Called = [erlang:module_loaded(on_load_call), on_load_call:value(),
              erlang:module_loaded(on_load_call)],
    io:format("~p~n", [[Ensured, Called]]).

%% Write the results of code_server for modules with an -on_load function
%% that fails: on_load_error returns an error, and on_load_crash raises.
%% code_server drops the code through erts_code_purger, and the load gives
%% {error, on_load_failure} (code_server.erl:1360-1368).
failure() ->
    Failed = [code:ensure_loaded(on_load_error), erlang:module_loaded(on_load_error),
              try on_load_crash:value() catch Class:Reason -> {Class, Reason} end,
              erlang:module_loaded(on_load_crash)],
    io:format("~p~n", [Failed]).
