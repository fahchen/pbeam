-module(undef_init).
-export([start/0]).

%% An init process for a run without a boot. `error_handler:ensure_loaded/1`
%% asks the registered init process for a module when no code server runs
%% (`error_handler.erl:182-198`). ERTS always has that process; a run of the
%% library API has it only when `pbeam_boot` starts it. This process answers
%% the requests of `init:ensure_loaded/1` (`init.erl:463-464`, `730-750`,
%% `1614-1624`) as the boot does, so the fixtures of this test compare the
%% error handler path with ERTS. It takes the name only when it is free, so
%% the ERTS run of a fixture keeps its own init process, and it ends at the
%% message `stop`.
start() ->
    Caller = self(),
    spawn(fun() -> serve(Caller) end),
    receive {undef_init, started} -> ok end.

serve(Caller) ->
    case register(init, self()) of
        true ->
            Caller ! {undef_init, started},
            loop();
        false ->
            ok
    end.

loop() ->
    receive
        {From, {ensure_loaded, Module}} ->
            From ! {init, ensure_loaded(Module)},
            loop();
        {From, fetch_loaded} ->
            From ! {init, []},
            loop();
        {From, get_arguments} ->
            From ! {init, []},
            loop();
        {From, {get_argument, _Arg}} ->
            From ! {init, error},
            loop();
        stop ->
            ok
    end.

%% The reply of `init:ensure_loaded/2` (`init.erl:730-750`): a loaded module
%% needs no file, and a module that this run cannot load gives the atom
%% `error` of `erl_prim_loader:get_file/1`.
ensure_loaded(Module) ->
    case erlang:module_loaded(Module) of
        true -> {module, Module};
        false -> error
    end.
