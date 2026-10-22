-module(code_loading).
-export([loader/0, caller/0, reference/0]).

%% The two roots load code_target on one scheduler and call it from the
%% other. code_target.hrl holds the stripped code of this module, compiled
%% with -DVERSION=1 and -DVERSION=2:
%%
%%   -module(code_target).
%%   -export([version/0, wait/1]).
%%   version() -> ?VERSION.
%%   wait(From) -> From ! waiting, receive go -> {old, version()} end.
-include("code_target.hrl").

%% The root that loads version 1, waits until the caller runs it, deletes
%% it, and loads version 2 while the caller still runs version 1.
loader() ->
    false = erlang:module_loaded(code_target),
    {module, code_target} = erlang:load_module(code_target, ?TARGET_V1),
    Caller = wait_registered(code_caller),
    Caller ! {loaded, self()},
    receive waiting -> ok end,
    true = erlang:delete_module(code_target),
    {module, code_target} = erlang:load_module(code_target, ?TARGET_V2),
    NotPurged = {error, not_purged} = erlang:load_module(code_target, ?TARGET_V1),
    Caller ! go,
    {NotPurged, code_target:version()}.

%% The root that calls version 1 of the other scheduler, waits in it, and
%% calls version 2 after the load.
caller() ->
    true = register(code_caller, self()),
    Loader = receive {loaded, From} -> From end,
    First = {erlang:module_loaded(code_target), code_target:version()},
    Old = {old, _} = code_target:wait(Loader),
    {First, Old, code_target:version()}.

%% The same two roots on the pinned ERTS.
reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {caller, caller()} end),
    _ = spawn(fun() -> Self ! {loader, loader()} end),
    Loader = receive {loader, L} -> L after 60000 -> timeout end,
    Caller = receive {caller, C} -> C after 60000 -> timeout end,
    {Loader, Caller}.

wait_registered(Name) ->
    case whereis(Name) of
        undefined ->
            erlang:yield(),
            wait_registered(Name);
        Pid ->
            Pid
    end.
