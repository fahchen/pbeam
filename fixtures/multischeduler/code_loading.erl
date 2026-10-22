-module(code_loading).
-export([loader/0, caller/0, reference/0, load_tell/0, apply_loaded/0]).

%% The two roots load code_target on one scheduler and call it from the
%% other. code_target.hrl holds the stripped code of this module, compiled
%% with -DVERSION=1 and -DVERSION=2:
%%
%%   -module(code_target).
%%   -export([version/0, wait/1]).
%%   version() -> ?VERSION.
%%   wait(From) -> From ! {waiting, self()}, receive go -> {old, version()} end.
-include("code_target.hrl").

%% The root that loads version 1, waits until a process of the caller runs
%% it and the caller is ready, deletes it, and loads version 2 while that
%% process still runs version 1.
loader() ->
    false = erlang:module_loaded(code_target),
    {module, code_target} = erlang:load_module(code_target, ?TARGET_V1),
    Caller = wait_registered(code_caller),
    Caller ! {loaded, self()},
    Waiter = receive {waiting, Pid} -> Pid end,
    receive ready -> ok end,
    true = erlang:delete_module(code_target),
    {module, code_target} = erlang:load_module(code_target, ?TARGET_V2),
    NotPurged = {error, not_purged} = erlang:load_module(code_target, ?TARGET_V1),
    Waiter ! go,
    Caller ! go,
    {NotPurged, code_target:version()}.

%% The root that calls version 1 of the other scheduler, starts a process
%% that waits in it, and calls version 2 after the load from the same
%% function, whose import of code_target:version/0 found version 1 before.
caller() ->
    true = register(code_caller, self()),
    Loader = receive {loaded, From} -> From end,
    First = {erlang:module_loaded(code_target), code_target:version()},
    Self = self(),
    _ = spawn(fun() -> Self ! {old, code_target:wait(Loader)} end),
    Loader ! ready,
    ok = poll(),
    Second = code_target:version(),
    Old = receive {old, Result = {old, _}} -> Result end,
    {First, Old, Second}.

%% Wait for go with a receive that does not suspend, so that the caller
%% finds the message while it runs.
poll() ->
    receive go -> ok after 0 -> poll() end.

%% The same two roots on the pinned ERTS.
reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {caller, caller()} end),
    _ = spawn(fun() -> Self ! {loader, loader()} end),
    Loader = receive {loader, L} -> L after 60000 -> timeout end,
    Caller = receive {caller, C} -> C after 60000 -> timeout end,
    {Loader, Caller}.

%% The root that loads code_target and gives an external fun and the module
%% name to the other root.
load_tell() ->
    {module, code_target} = erlang:load_module(code_target, ?TARGET_V1),
    wait_registered(code_asker) ! {fun erlang:module_loaded/1, code_target},
    ok.

%% The root that calls the external fun and apply/3 with the module that the
%% other root loaded.
apply_loaded() ->
    true = register(code_asker, self()),
    receive {Loaded, Module} -> {Loaded(Module), apply(Module, version, [])} end.

wait_registered(Name) ->
    case whereis(Name) of
        undefined ->
            erlang:yield(),
            wait_registered(Name);
        Pid ->
            Pid
    end.
