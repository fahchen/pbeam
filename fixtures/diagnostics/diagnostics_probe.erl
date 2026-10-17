-module(diagnostics_probe).
-export([main/0, display/0, display_bad/0, alias_priority/0, delete_old/0, prepared/2]).

%% Return the class and the reason of a raised term.
t(F) ->
    try F() catch Class:Reason -> {Class, Reason} end.

%% Return the kind of a value, so that the two runs compare the item shapes
%% of erlang:process_info/1 rather than the live values.
kind(V) when is_atom(V) -> atom;
kind(V) when is_integer(V) -> integer;
kind(V) when is_list(V) -> list;
kind(V) when is_tuple(V) -> tuple;
kind(V) when is_pid(V) -> pid;
kind(V) when is_reference(V) -> reference;
kind(V) when is_binary(V) -> binary;
kind(V) when is_map(V) -> map.

%% Delete a loaded module and a module that no run has, and a term that is
%% not an atom.
delete_case() ->
    ok = diagnostics_target:id(ok),
    Loaded = erlang:module_loaded(diagnostics_target),
    First = erlang:delete_module(diagnostics_target),
    After = erlang:module_loaded(diagnostics_target),
    Absent = erlang:delete_module(diagnostics_absent_module),
    Bad = t(fun() -> erlang:delete_module(1) end),
    {Loaded, First, After, Absent, Bad}.

%% A second delete of the module gives badarg, because the code is old code
%% until a purge. ERTS also writes a logger report, which PBeam does not.
delete_old() ->
    ok = diagnostics_target:id(ok),
    true = erlang:delete_module(diagnostics_target),
    t(fun() -> erlang:delete_module(diagnostics_target) end).

%% Read the items of a waiting process and of a runnable process. The item
%% keys and the kind of each value do not depend on the state of that
%% process.
other_process_info() ->
    Waiting = spawn(fun() ->
                            receive {ping, Parent} ->
                                    Parent ! pong,
                                    receive stop -> ok end
                            end
                    end),
    Waiting ! {ping, self()},
    receive pong -> ok end,
    Runnable = spawn(fun() -> receive stop -> ok end end),
    Info = [erlang:process_info(Waiting), erlang:process_info(Runnable)],
    Shapes = [{Keys, [kind(Value) || {_, Value} <- Items]}
              || Items <- Info,
                 Keys <- [[Key || {Key, _} <- Items]]],
    Waiting ! stop,
    Runnable ! stop,
    Shapes.

%% End the alias of a monitor that has an alias option. ERTS keeps the
%% monitor, and a later demonitor finds it.
alias_monitor() ->
    Child = spawn(fun() -> receive stop -> ok end end),
    Monitor = erlang:monitor(process, Child, [{alias, explicit_unalias}]),
    Ended = erlang:unalias(Monitor),
    Again = erlang:unalias(Monitor),
    Kept = erlang:demonitor(Monitor),
    Child ! stop,
    {Ended, Again, Kept}.

%% Read a pid after its process exited.
dead_process_info() ->
    Pid = spawn(fun() -> ok end),
    Reference = monitor(process, Pid),
    receive {'DOWN', Reference, process, Pid, _} -> ok end,
    erlang:process_info(Pid).

%% The diagnostics BIFs of the plug_hello release. The alias, unique
%% integer, and scheduler values are relations, because ERTS and PBeam start
%% with different counter and scheduler values.
main() ->
    process_flag(trap_exit, true),
    put(diagnostics_key, value),
    true = register(diagnostics_probe_name, self()),
    Info = erlang:process_info(self()),
    Keys = [Key || {Key, _} <- Info],
    Kinds = [kind(Value) || {_, Value} <- Info],
    Registered = erlang:registered(),
    RegisteredShape = [lists:member(diagnostics_probe_name, Registered),
                       lists:all(fun erlang:is_atom/1, Registered)],
    Ports = erlang:ports(),
    PortShape = [is_list(Ports), lists:all(fun erlang:is_port/1, Ports)],
    Alias = erlang:alias([explicit_unalias]),
    AliasShape = [is_reference(Alias), is_reference(erlang:alias()),
                  is_reference(erlang:alias([reply])),
                  erlang:unalias(Alias), erlang:unalias(Alias),
                  erlang:unalias(make_ref())],
    AliasBad = [t(fun() -> erlang:alias(1) end),
                t(fun() -> erlang:alias([bogus]) end),
                t(fun() -> erlang:alias([explicit_unalias | a]) end),
                t(fun() -> erlang:unalias(1) end)],
    M1 = erlang:unique_integer([monotonic]),
    M2 = erlang:unique_integer([monotonic]),
    MP = erlang:unique_integer([monotonic, positive]),
    N1 = erlang:unique_integer([positive]),
    N2 = erlang:unique_integer([positive]),
    U0 = erlang:unique_integer([]),
    UniqueShape = [M1 < 0, M2 =:= M1 + 1, MP > 0, N1 > 0, N2 > N1,
                   U0 < 0],
    UniqueBad = [t(fun() -> erlang:unique_integer([negative]) end),
                 t(fun() -> erlang:unique_integer([bogus]) end),
                 t(fun() -> erlang:unique_integer([positive | a]) end),
                 t(fun() -> erlang:unique_integer(positive) end)],
    Schedulers = erlang:system_info(schedulers_online) =:=
                     erlang:system_info(schedulers),
    Ref = erlang:ref_to_list(make_ref()),
    RefShape = [lists:prefix("#Ref<", Ref), is_list(Ref),
                t(fun() -> erlang:ref_to_list(1) end)],
    InfoBad = t(fun() -> erlang:process_info(1) end),
    {Keys, Kinds, RegisteredShape, PortShape, AliasShape, AliasBad,
     UniqueShape, UniqueBad, erts_internal:time_unit(), Schedulers, RefShape, InfoBad,
     delete_case(), error_handler_info(), other_process_info(), alias_monitor(),
     dead_process_info()}.

%% Read the error_handler item with a custom handler.
error_handler_info() ->
    Old = process_flag(error_handler, diagnostics_handler),
    Info = erlang:process_info(self()),
    process_flag(error_handler, Old),
    element(2, lists:keyfind(error_handler, 1, Info)).

%% The badarg cases of display_string/2.
display_bad() ->
    [t(fun() -> erlang:display_string(foo, "x") end),
     t(fun() -> erlang:display_string(stdout, 1) end),
     t(fun() -> erlang:display_string(stdout, [a]) end),
     t(fun() -> erlang:display_string(stdout, [16#D800]) end),
     t(fun() -> erlang:display_string(stdout, [16#110000]) end),
     t(fun() -> erlang:display_string(stdout, [111, 107 | bad]) end),
     t(fun() -> erlang:display_string(1, "x") end)].

%% PBeam raises a runtime error for the priority alias option.
alias_priority() ->
    erlang:alias([priority]).

%% Prepare one module without an -on_load function and one with it, and
%% test has_prepared_code_on_load/1 on each reference.
prepared(PlainCode, OnLoadCode) ->
    Plain = erts_internal:prepare_loading(diagnostics_target, PlainCode),
    OnLoad = erts_internal:prepare_loading(diagnostics_on_load, OnLoadCode),
    {erlang:has_prepared_code_on_load(Plain),
     erlang:has_prepared_code_on_load(OnLoad),
     t(fun() -> erlang:has_prepared_code_on_load(make_ref()) end),
     t(fun() -> erlang:has_prepared_code_on_load(1) end)}.

%% The output of display/1 and display_string/2. The test compares the
%% standard output and the standard error of the two runs.
display() ->
    true = erlang:display({hello, "world"}),
    true = erlang:display_string(stdout, "out"),
    true = erlang:display_string(stderr, <<"err">>),
    true = erlang:display_string(stdout, [16#1F600, $\n]),
    ok.
