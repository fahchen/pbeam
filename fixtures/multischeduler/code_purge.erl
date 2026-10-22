-module(code_purge).
-export([loader/0, holder/0, reference/0]).

%% The two roots purge code on one scheduler while a process of the other
%% scheduler runs it. purge_target.hrl holds the stripped code of this
%% module:
%%
%%   -module(purge_target).
%%   -export([wait/1]).
%%   -on_load(init/0).
%%   init() ->
%%       purge_holder ! {run, self(), fun wait/1},
%%       receive running -> error end.
%%   wait(From) ->
%%       From ! {waiting, self()},
%%       receive stop -> ok end.
-include("purge_target.hrl").

%% The root that loads purge_target, whose -on_load function gives a fun of
%% the module to the holder and fails after a process of the holder runs
%% the fun. erts_code_purger then purges the code and kills that process.
%% Then the root purges modules with no old code.
loader() ->
    ok = ensure_purger(),
    _ = wait_registered(purge_holder),
    {error, on_load} = erlang:load_module(purge_target, ?PURGE_TARGET),
    Init = erlang:call_on_load_function(purge_target),
    ok = erts_code_purger:finish_after_on_load(purge_target, false),
    {Init, erlang:module_loaded(purge_target),
     erts_code_purger:purge(purge_target),
     erts_code_purger:soft_purge(purge_target),
     erts_code_purger:purge(code_purge),
     erts_code_purger:soft_purge(code_purge)}.

%% The root that starts a process in the fun of the -on_load function and
%% gives the reason of its exit.
holder() ->
    true = register(purge_holder, self()),
    {Init, Wait} = receive {run, From, Fun} -> {From, Fun} end,
    Self = self(),
    {Pid, Ref} = spawn_monitor(fun() -> Wait(Self) end),
    receive {waiting, Pid} -> ok end,
    Init ! running,
    receive {'DOWN', Ref, process, Pid, Reason} -> {down, Reason} end.

%% The same two roots on the pinned ERTS.
reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {holder, holder()} end),
    _ = spawn(fun() -> Self ! {loader, loader()} end),
    Loader = receive {loader, L} -> L after 60000 -> timeout end,
    Holder = receive {holder, H} -> H after 60000 -> timeout end,
    {Loader, Holder}.

%% A run of execute_on_schedulers has no boot, so the loader starts the
%% pinned erts_code_purger when no process has its name.
ensure_purger() ->
    case whereis(erts_code_purger) of
        undefined ->
            _ = spawn(erts_code_purger, start, []),
            _ = wait_registered(erts_code_purger),
            ok;
        _ ->
            ok
    end.

wait_registered(Name) ->
    case whereis(Name) of
        undefined ->
            erlang:yield(),
            wait_registered(Name);
        Pid ->
            Pid
    end.
