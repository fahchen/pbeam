-module(load_probe).
-export([load/1, prepared/1, errors/1, replace/1, on_load/1, order/1, constraint/1, late/1,
         on_load_ok/1, on_load_error/1, on_load_crash/1, call_on_load/1, unsupported/1,
         keep_after_drop/1,
         undefined_function/3]).

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

%% Run the same external calls before and after the load of load_target.
%% Each call_ext names an export entry, which the ERTS loader patches into
%% the code (jit/asm_load.c:1198-1206). Before the load, the entry of each
%% function calls the error handler of the process
%% (call_error_handler, beam_common.c:1294-1342). The load sets the address
%% of the entries of the exported functions, so the same call sites then run
%% load_target code. load_target:missing/0 stays undefined after the load.
%% This module is the error handler of the calls, so the run needs no code
%% server. erlang:apply/2,3 call through the export too.
late(Code) ->
    Before = handled(fun calls/0),
    Loaded = erlang:load_module(load_target, Code),
    After = handled(fun calls/0),
    {Before, Loaded, After}.

handled(Fun) ->
    Old = erlang:process_flag(error_handler, load_probe),
    Result = Fun(),
    erlang:process_flag(error_handler, Old),
    Result.

calls() ->
    [ext_only(), ext_only(), ext_body(), ext_last(), missing(), missing(),
     apply_mfa(load_target, value, []), apply_mfa(load_target, missing, []),
     apply_fun(fun load_target:value/0, []), [apply_mfa_body(load_target, value, [])]].

ext_only() -> load_target:value().

ext_body() -> [load_target:value()].

ext_last() ->
    _ = id(a),
    load_target:value().

missing() -> load_target:missing().

apply_mfa(Module, Function, Arguments) -> erlang:apply(Module, Function, Arguments).

apply_mfa_body(Module, Function, Arguments) -> [erlang:apply(Module, Function, Arguments)].

apply_fun(Fun, Arguments) -> erlang:apply(Fun, Arguments).

id(X) -> X.

%% The error handler of late/1: return the call that has no export.
undefined_function(Module, Function, Arguments) ->
    {undefined, Module, Function, Arguments}.

%% Run the on_load steps of code_server and erts_code_purger for the
%% modules of fixtures/on_load (on_load_steps/2).
on_load_ok(Code) -> on_load_steps(on_load_ok, Code).

on_load_error(Code) -> on_load_steps(on_load_error, Code).

on_load_crash(Code) -> on_load_steps(on_load_crash, Code).

%% Load Module, which has an -on_load function, from its BEAM bytes, and
%% run its on_load function in a new process, as code_server does
%% (code_server.erl:1348-1352, 1386-1398). Then keep or drop the code with
%% erlang:finish_after_on_load/2, as erts_code_purger does
%% (erts_code_purger.erl:166-167). Read the module and its export before
%% and after the steps, with the error handler of late/1, and the error
%% cases of the BIFs: a group of more than one module, a keep value that
%% is not a boolean, a module with no on_load function, a module that is
%% not an atom, and a call of
%% erlang:call_on_load_function/1 through apply/3 with variable names.
on_load_steps(Module, Code) ->
    Prepared = erlang:prepare_loading(Module, Code),
    Group = catch_error(fun() -> erlang:finish_loading([Prepared, Prepared]) end),
    Finish = erlang:finish_loading([Prepared]),
    Pending = on_load_state(Module),
    BadKeep = catch_error(fun() -> erlang:finish_after_on_load(Module, keep) end),
    Missing = catch_error(fun() -> erlang:finish_after_on_load(on_load_missing, true) end),
    NoOnLoad = [catch_error(fun() -> erlang:call_on_load_function(load_probe) end),
                catch_error(fun() -> erlang:call_on_load_function(id(1)) end),
                catch_error(fun() -> erlang:finish_after_on_load(id(1), true) end)],
    Applied = catch_error(fun() -> erlang:apply(id(erlang), id(call_on_load_function), [Module]) end),
    {Pid, Ref} = spawn_monitor(load_probe, call_on_load, [Module]),
    Result = receive {'DOWN', Ref, process, Pid, Reason} -> without_locations(Reason) end,
    Keep = Result =:= ok,
    Done = erlang:finish_after_on_load(Module, Keep),
    After = on_load_state(Module),
    Again = catch_error(fun() -> erlang:finish_after_on_load(Module, Keep) end),
    Call = catch_error(fun() -> erlang:call_on_load_function(Module) end),
    {Group, Finish, Pending, BadKeep, Missing, NoOnLoad, Applied, Result, Done, After, Again,
     Call}.

%% The process of code_server that runs the on_load function
%% (code_server.erl:1391-1394).
call_on_load(Module) ->
    exit(erlang:call_on_load_function(Module)).

%% Drop the code of on_load_ok after its on_load function, and then keep
%% it. ERTS makes the code current with exports that call the error handler
%% (beam_bif_load.c:887-897).
keep_after_drop(Code) ->
    {on_load, [on_load_ok]} = erlang:finish_loading([erlang:prepare_loading(on_load_ok, Code)]),
    true = erlang:finish_after_on_load(on_load_ok, false),
    erlang:finish_after_on_load(on_load_ok, true).

%% Prepare a module with an instruction that the PBeam loader does not
%% decode.
unsupported(Code) ->
    erlang:prepare_loading(load_unsupported, Code).

%% Remove the locations of the stack trace of an exit reason, which PBeam
%% does not give (row "Exception stack trace" of docs/erts-differences.md).
without_locations({Reason, Stack}) when is_list(Stack) ->
    {Reason, [{M, F, A} || {M, F, A, _} <- Stack]};
without_locations(Reason) ->
    Reason.

on_load_state(Module) ->
    [erlang:module_loaded(Module), erlang:function_exported(Module, value, 0),
     handled(fun() -> Module:value() end)].

catch_error(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.
