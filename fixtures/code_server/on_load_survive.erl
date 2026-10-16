-module(on_load_survive).
-export([value/0]).
-on_load(init/0).

%% The on_load function starts a process that leaves the code of this
%% module at once, so no frame of the process is in the code, and the
%% check_process_code request of erts_code_purger answers false. The purger
%% does not kill the process (erts_code_purger.erl:166-182, 249-265), and
%% the process keeps the module and the function of its start MFA after the
%% purge removes the code (beam_bif_load.c:2249-2262).
init() ->
    Pid = spawn(fun leave/0),
    on_load_spawn_probe ! {waiting, Pid},
    error.

leave() ->
    on_load_server_probe:wait().

value() ->
    on_load_survive_value.
