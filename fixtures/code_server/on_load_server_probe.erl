-module(on_load_server_probe).
-export([main/0, failure/0, hibernated/0]).

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

%% Write the result of an on_load load through an erts_code_purger that
%% hibernated as its idle timeout does: after 60 seconds with no request,
%% wait_for_request/0 calls erlang:hibernate(erts_code_purger,
%% wait_for_request, []) (erts_code_purger.erl:40-45). The probe starts a
%% process that makes the same call, lets it run to the call with one
%% scheduler, and registers it as erts_code_purger. code_server then makes
%% the code of on_load_ok current through it (erts_code_purger.erl:55-59,
%% 166-170).
hibernated() ->
    Probe = self(),
    Purger = spawn(fun() ->
                           Probe ! hibernating,
                           erlang:hibernate(erts_code_purger, wait_for_request, [])
                   end),
    receive hibernating -> ok end,
    erlang:yield(),
    true = unregister(erts_code_purger),
    true = register(erts_code_purger, Purger),
    Loaded = [code:ensure_loaded(on_load_ok), on_load_ok:value(),
              erlang:is_process_alive(Purger)],
    io:format("~p~n", [Loaded]).
