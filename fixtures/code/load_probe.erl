-module(load_probe).
-export([load/1, prepared/1, errors/1, replace/1, on_load/1, order/1, constraint/1]).

%% Load load_target from its BEAM bytes with erlang:load_module/2
%% (erlang.erl:7513-7528), then call it. Read erlang:module_loaded/1 and
%% erlang:function_exported/3 before and after the load.
load(Code) ->
    Before = {erlang:module_loaded(load_target), erlang:function_exported(load_target, value, 0)},
    Loaded = erlang:load_module(load_target, Code),
    After = {erlang:module_loaded(load_target), erlang:function_exported(load_target, value, 0)},
    Value = load_target:value(),
    %% The name comes from the result, so the compiler does not fold the
    %% list_to_atom/1 call into a literal atom of this module.
    Name = atom_to_list(element(2, Loaded)) ++ "_value",
    {Before, Loaded, After, Value, Value =:= list_to_atom(Name)}.

%% Prepare load_target two times, and finish the prepared code. A list with
%% two prepared values of one module returns {duplicated, Modules} and keeps
%% each prepared value (beam_bif_load.c:429-441). A prepared value that
%% finished is not prepared code any more, so it raises badarg
%% (beam_bif_load.c:385-392).
prepared(Code) ->
    Prepared = erlang:prepare_loading(load_target, Code),
    Again = erlang:prepare_loading(load_target, Code),
    [erlang:is_reference(Prepared),
     erlang:finish_loading([Prepared, Again]),
     erlang:finish_loading([Prepared, Again, Prepared]),
     erlang:module_loaded(load_target),
     erlang:finish_loading([]),
     erlang:finish_loading([Prepared]),
     erlang:module_loaded(load_target),
     load_target:value(),
     catch_error(fun() -> erlang:finish_loading([Prepared]) end)].

%% Give each operation arguments that are not valid. Bytes that do not start
%% with FOR1 go through zlib:gunzip/1 in erlang:prepare_loading/2
%% (erlang.erl:5436-5447), which raises for them, and then to
%% erts_internal:prepare_loading/2.
errors(Code) ->
    [erlang:load_module(load_target, <<"bad">>),
     erlang:load_module(load_target, <<"FOR1bad">>),
     erlang:load_module(load_probe_other, Code),
     erts_internal:prepare_loading(load_target, <<"bad">>),
     erts_internal:prepare_loading(load_target, <<>>),
     catch_error(fun() -> erlang:finish_loading([a]) end),
     catch_error(fun() -> erlang:finish_loading(a) end),
     catch_error(fun() -> erlang:finish_loading([a | b]) end),
     catch_error(fun() -> erlang:finish_loading([make_ref()]) end),
     catch_error(fun() -> erts_internal:prepare_loading("load_target", Code) end),
     catch_error(fun() -> erts_internal:prepare_loading(load_target, "FOR1") end),
     catch_error(fun() -> erlang:prepare_loading(1, Code) end),
     catch_error(fun() -> erlang:load_module(1, Code) end),
     erlang:module_loaded(load_target),
     erlang:module_loaded(load_probe_other)].

%% Load load_target two times. The second load replaces the current code.
replace(Code) ->
    [erlang:load_module(load_target, Code), erlang:load_module(load_target, Code)].

%% Load a module that has an -on_load function.
on_load(Code) ->
    [erlang:load_module(load_on_load, Code), erlang:module_loaded(load_on_load)].

%% Spawn a process that sends a message, and then load load_target. The
%% caller of erlang:finish_loading/1 switches out until the new code is
%% committed (beam_bif_load.c:555-566), so the new process runs first.
order(Code) ->
    Self = self(),
    spawn(fun() -> Self ! spawned end),
    Loaded = erlang:load_module(load_target, Code),
    Self ! loaded,
    [Loaded, receive First -> First end, receive Second -> Second end].

%% Prepare load_target and finish it with a list of one module. The `modules`
%% constraint of the `code` `load` capability compares the module name of
%% each element of the list.
constraint(Code) ->
    Prepared = erlang:prepare_loading(load_target, Code),
    try erlang:finish_loading([Prepared]) of
        ok -> ok
    catch
        error:capability_denied -> denied
    end.

catch_error(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.
