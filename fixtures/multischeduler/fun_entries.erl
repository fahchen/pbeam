-module(fun_entries).
-export([single/0, loader/0, maker/0, reference/0]).

%% Local funs of two versions of fun_target, whose fun entries the loader
%% keeps in the fun-entry table of the node. fun_target.hrl holds the
%% stripped code of this module, compiled with -DVERSION=1 and -DVERSION=2:
%%
%%   -module(fun_target).
%%   -export([make/1]).
%%   make(X) -> fun() -> {?VERSION, X} end.
-include("fun_target.hrl").

%% Load version 1 and, after a delete, version 2, and make funs of each.
single() ->
    {module, fun_target} = erlang:load_module(fun_target, ?FUN_TARGET_V1),
    F1 = fun_target:make(a),
    true = erlang:delete_module(fun_target),
    {module, fun_target} = erlang:load_module(fun_target, ?FUN_TARGET_V2),
    F2 = fun_target:make(a),
    [info(F1), info(F2), F1 == F2, F2 == fun_target:make(a), F2()].

%% The root that loads version 1, gets the fun that the other root makes on
%% its scheduler, and compares it with a fun that it makes itself.
loader() ->
    {module, fun_target} = erlang:load_module(fun_target, ?FUN_TARGET_V1),
    wait_registered(fun_maker) ! {loaded, self()},
    F = receive {made, G} -> G end,
    {info(F), F == fun_target:make(a), F()}.

%% The root that makes a fun of the code that the other root loaded.
maker() ->
    true = register(fun_maker, self()),
    Loader = receive {loaded, From} -> From end,
    F = fun_target:make(a),
    Loader ! {made, F},
    info(F).

%% single/0 and the two roots on the pinned ERTS.
reference() ->
    Single = single(),
    {_, _} = erts_code_purger:purge(fun_target),
    true = erlang:delete_module(fun_target),
    {_, _} = erts_code_purger:purge(fun_target),
    Self = self(),
    _ = spawn(fun() -> Self ! {maker, maker()} end),
    _ = spawn(fun() -> Self ! {loader, loader()} end),
    Loader = receive {loader, L} -> L after 60000 -> timeout end,
    Maker = receive {maker, M} -> M after 60000 -> timeout end,
    {Single, Loader, Maker}.

info(F) ->
    [erlang:fun_info(F, I) || I <- [index, new_index, uniq, new_uniq, pid]].

wait_registered(Name) ->
    case whereis(Name) of
        undefined ->
            erlang:yield(),
            wait_registered(Name);
        Pid ->
            Pid
    end.
