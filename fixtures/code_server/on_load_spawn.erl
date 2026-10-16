-module(on_load_spawn).
-export([value/0]).
-on_load(init/0).

%% The on_load function starts a process that waits in the code of this
%% module, sends its pid to the process registered as on_load_spawn_probe,
%% and fails. erts_code_purger finds that the process refers to the code
%% and kills it before it purges the code (erts_code_purger.erl:166-182,
%% 249-265).
init() ->
    Pid = spawn(fun wait/0),
    on_load_spawn_probe ! {waiting, Pid},
    error.

wait() ->
    receive
        stop -> ok
    end.

value() ->
    on_load_spawn_value.
