-module(effect_probe).

-include_lib("kernel/include/file.hrl").
-compile({nowarn_deprecated_function, [{erlang, now, 0}]}).
-export([
    spawn/0,
    spawn_system_process/0,
    spawn_link/0,
    spawn_opt/0,
    spawn_opt_link/0,
    spawn_opt_monitor/0,
    open_port/0,
    port_command/0,
    port_close/0,
    port_info/0,
    port_info2/0,
    port_connect/0,
    port_badargs/0,
    port_control/0,
    port_set_data/0,
    port_get_data/0,
    getenv/0,
    env/0,
    putenv/0,
    unsetenv/0,
    handle_signal/0,
    ignore_signal/0,
    default_signal/0,
    write_nif/0,
    read_nif/0,
    tty_select/0,
    tty_init/0,
    tty_window_size/0,
    isatty/0,
    tty_create/0,
    setlocale/0,
    setupterm/0,
    tigetnum/0,
    tigetflag/0,
    halt_run/0,
    system_flag/0,
    load_nif/0,
    link/0,
    monitor2/0,
    monitor3/0,
    exit_other/0,
    exit_other3/0,
    send_instruction/0,
    send2/0,
    send3/0,
    start_timer/0,
    start_timer_4/0,
    send_after/0,
    send_after_4/0,
    process_info/0,
    process_info1/0,
    delete_module/0,
    is_process_alive/0,
    is_alive_request/0,
    group_leader2/0,
    group_leader3/0,
    pt_get0/0,
    pt_get1/0,
    pt_get2/0,
    pt_put/0,
    pt_put_new/0,
    pt_info/0,
    pt_erase/0,
    permitted/0,
    runtime_permitted/0,
    runtime_badargs/0,
    system_flag_kept/0,
    load_nif_kept/0,
    process_permitted/0,
    process_badargs/0,
    link_kept/0,
    monitor_kept/0,
    exit_kept/0,
    exit_self/0,
    send_kept/0,
    timer_kept/0,
    process_inspect_permitted/0,
    process_inspect_badargs/0,
    process_inspect_self/0,
    inspect_kept/0,
    group_leader_kept/0,
    persistent_term_permitted/0,
    persistent_term_kept/0,
    terminal_permitted/0,
    terminal_badargs/0,
    inspect_permitted/0,
    inspect_closed/0,
    inspect_names/0,
    module_loaded/0,
    loaded/0,
    get_module_info/0,
    get_module_info_1/0,
    finish_loading/0,
    code_names/0,
    code_badarg/0,
    ambient/0,
    child/2,
    value_child/2,
    leader/0,
    names/0,
    executables/0,
    signals/0,
    close_kept/0,
    stop/0,
    done/0
]).

%% One call of each effect entry that checks a capability. Each function
%% gives `denied` for error:capability_denied. With the check missing, a
%% call has a small effect only: it reads no input, writes no byte, and
%% changes only the disposition of SIGUSR2.

spawn() -> attempt(fun() -> erlang:spawn(?MODULE, done, []) end).

%% Each test runs it without `spawn`: a system process that exits stops the
%% run (erl_process.c:14186-14188).
spawn_system_process() ->
    attempt(fun() -> erts_internal:spawn_system_process(?MODULE, done, []) end).

spawn_link() -> attempt(fun() -> erlang:spawn_link(?MODULE, done, []) end).

spawn_opt() -> attempt(fun() -> erlang:spawn_opt(?MODULE, done, [], []) end).

spawn_opt_link() -> attempt(fun() -> erlang:spawn_opt(?MODULE, done, [], [link]) end).

spawn_opt_monitor() -> attempt(fun() -> erlang:spawn_opt(?MODULE, done, [], [monitor]) end).

open_port() -> attempt(fun() -> cat() end).

port_command() ->
    Port = cat(),
    attempt(fun() -> erlang:port_command(Port, <<"x">>) end).

port_close() ->
    Port = cat(),
    attempt(fun() -> erlang:port_close(Port) end).

port_info() ->
    Port = cat(),
    attempt(fun() -> erlang:port_info(Port) end).

port_info2() ->
    Port = cat(),
    attempt(fun() -> erlang:port_info(Port, connected) end).

port_connect() ->
    Port = cat(),
    attempt(fun() -> erlang:port_connect(Port, self()) end).

%% A port of the spawn driver has no control, so a permitted call raises
%% badarg (io.c:4029-4030).
port_control() ->
    Port = cat(),
    attempt(fun() -> erlang:port_control(Port, 1, []) end).

port_set_data() ->
    Port = cat(),
    attempt(fun() -> erlang:port_set_data(Port, data) end).

port_get_data() ->
    Port = cat(),
    attempt(fun() -> erlang:port_get_data(Port) end).

getenv() -> attempt(fun() -> os:getenv("PBEAM_EFFECT_PROBE") end).

env() -> attempt(fun() -> os:env() end).

putenv() -> attempt(fun() -> os:putenv("PBEAM_EFFECT_PROBE", "x") end).

unsetenv() -> attempt(fun() -> os:unsetenv("PBEAM_EFFECT_PROBE") end).

handle_signal() -> attempt(fun() -> os:set_signal(sigusr2, handle) end).

ignore_signal() -> attempt(fun() -> os:set_signal(sigusr2, ignore) end).

default_signal() -> attempt(fun() -> os:set_signal(sigusr2, default) end).

write_nif() ->
    {ok, TTY} = prim_tty:tty_create(stdout),
    attempt(fun() -> prim_tty:write_nif(TTY, []) end).

%% A read of 0 bytes does not wait for input.
read_nif() ->
    {ok, TTY} = prim_tty:tty_create(stdout),
    attempt(fun() -> prim_tty:read_nif(TTY, make_ref(), 0) end).

tty_select() ->
    {ok, TTY} = prim_tty:tty_create(stdout),
    attempt(fun() -> prim_tty:tty_select(TTY, make_ref()) end).

tty_init() ->
    {ok, TTY} = prim_tty:tty_create(stdout),
    attempt(fun() -> prim_tty:tty_init(TTY, #{input => cooked}) end).

tty_window_size() ->
    {ok, TTY} = prim_tty:tty_create(stdout),
    attempt(fun() -> prim_tty:tty_window_size(TTY) end).

isatty() -> attempt(fun() -> prim_tty:isatty(stdout) end).

tty_create() -> attempt(fun() -> prim_tty:tty_create(stdout) end).

%% A permitted setlocale/1 sets the locale of the OS process. The NIF does
%% not read its argument.
setlocale() -> attempt(fun() -> prim_tty:setlocale(tty) end).

setupterm() -> attempt(fun() -> prim_tty:setupterm_nif() end).

tigetnum() -> attempt(fun() -> prim_tty:tigetnum_nif("co\0") end).

tigetflag() -> attempt(fun() -> prim_tty:tigetflag_nif("xn\0") end).

%% A permitted halt stops the run with the status 3.
halt_run() -> attempt(fun() -> erlang:halt(3, []) end).

%% `logger` is the start value of the system logger in PBeam.
system_flag() -> attempt(fun() -> erlang:system_flag(system_logger, logger) end).

%% The stand-in `prim_tty` calls erlang:load_nif/2 from its own code.
load_nif() -> attempt(fun() -> prim_tty:load("prim_tty") end).

%% The process entries. A link, a monitor, a send, and a timer to the
%% caller have a check too. An exit signal to the caller has no check, so
%% the exit goes to a child.
link() -> attempt(fun() -> erlang:link(self()) end).

monitor2() -> attempt(fun() -> erlang:monitor(process, self()) end).

monitor3() -> attempt(fun() -> erlang:monitor(process, self(), []) end).

exit_other() ->
    Child = erlang:spawn(?MODULE, done, []),
    attempt(fun() -> erlang:exit(Child, normal) end).

exit_other3() ->
    Child = erlang:spawn(?MODULE, done, []),
    attempt(fun() -> erlang:exit(Child, normal, [priority]) end).

%% `!` compiles to the send instruction, and erlang:send/2 to a call of the
%% BIF.
send_instruction() -> attempt(fun() -> self() ! ping end).

send2() -> attempt(fun() -> erlang:send(self(), ping) end).

send3() -> attempt(fun() -> erlang:send(self(), ping, []) end).

start_timer() -> attempt(fun() -> erlang:start_timer(0, self(), ping) end).

start_timer_4() -> attempt(fun() -> erlang:start_timer(0, self(), ping, []) end).

send_after() -> attempt(fun() -> erlang:send_after(0, self(), ping) end).

send_after_4() -> attempt(fun() -> erlang:send_after(0, self(), ping, []) end).

%% The process inspection and group leader entries. A call for the caller
%% has no check, so each probe calls them for a child.
process_info() -> other(fun(Pid) -> erlang:process_info(Pid, registered_name) end).

%% The diagnostics BIF for another process, which checks process:inspect.
process_info1() -> other(fun(Pid) -> erlang:process_info(Pid) end).

%% The code delete, which checks code:load.
delete_module() -> attempt(fun() -> erlang:delete_module(effect_probe) end).

is_process_alive() -> other(fun(Pid) -> erlang:is_process_alive(Pid) end).

is_alive_request() -> other(fun(Pid) -> erts_internal:is_process_alive(Pid, make_ref()) end).

group_leader2() -> other(fun(Pid) -> erts_internal:group_leader(group_leader(), Pid) end).

group_leader3() ->
    other(fun(Pid) -> erts_internal:group_leader(group_leader(), Pid, make_ref()) end).

other(Call) ->
    Child = erlang:spawn(?MODULE, done, []),
    attempt(fun() -> Call(Child) end).

%% The persistent term entries. The key has no value, so a get/1 without
%% the check raises badarg.
pt_get0() -> attempt(fun() -> persistent_term:get() end).

pt_info() -> attempt(fun() -> persistent_term:info() end).

pt_get1() -> attempt(fun() -> persistent_term:get({?MODULE, missing}) end).

pt_get2() -> attempt(fun() -> persistent_term:get({?MODULE, missing}, none) end).

pt_put() -> attempt(fun() -> persistent_term:put({?MODULE, put}, 1) end).

pt_put_new() -> attempt(fun() -> persistent_term:put_new({?MODULE, put_new}, 1) end).

pt_erase() -> attempt(fun() -> persistent_term:erase({?MODULE, put}) end).

%% Permitted calls that give the same result on ERTS. The variable
%% PBEAM_EFFECT_PROBE is not set.
permitted() ->
    Port = cat(),
    Command = erlang:port_command(Port, <<"x">>),
    [
        os:getenv("PBEAM_EFFECT_PROBE"),
        is_pid(erlang:spawn(?MODULE, done, [])),
        Command,
        echo(Port),
        erlang:port_close(Port),
        badarg(fun() -> os:getenv(foo) end),
        badarg(fun() -> erlang:port_close(Port) end),
        badarg(fun() -> os:set_signal(sigint, handle) end)
    ].

%% Permitted runtime calls that give the same result on ERTS with the
%% stand-in `prim_tty`. The first call makes the system logger the same:
%% its start value on ERTS is the pid of the logger proxy.
runtime_permitted() ->
    erlang:system_flag(system_logger, undefined),
    [
        erlang:system_flag(system_logger, logger),
        erlang:system_flag(debug_flags, 1),
        prim_tty:load("prim_tty"),
        prim_tty:load("prim_tty")
    ].

%% The argument checks come before the runtime capability check.
runtime_badargs() ->
    [
        badarg(fun() -> erlang:halt(-1, []) end),
        badarg(fun() -> erlang:halt(3, [foo]) end),
        badarg(fun() -> erlang:system_flag(foo, 1) end),
        badarg(fun() -> erlang:system_flag(system_logger, 1) end),
        badarg(fun() -> erlang:system_flag(max_heap_size, -1) end),
        badarg(fun() -> prim_tty:load(foo) end)
    ].

%% A child with a runtime capability of no operation tries the call. A
%% denied call has no effect, so the next call of the parent sees the
%% start state.
system_flag_kept() ->
    Denied = restricted(runtime, fun() -> erlang:system_flag(system_logger, undefined) end),
    {Denied, erlang:system_flag(system_logger, logger)}.

load_nif_kept() ->
    Denied = restricted(runtime, fun() -> prim_tty:load("prim_tty") end),
    {Denied, prim_tty:load("prim_tty")}.

%% Permitted persistent term calls that give the same result on ERTS. A put
%% of the same value again also gives ok.
persistent_term_permitted() ->
    Key = {?MODULE, permitted},
    Missing = persistent_term:get(Key, none),
    Badarg = badarg(fun() -> persistent_term:get(Key) end),
    Put = persistent_term:put(Key, 1),
    Same = persistent_term:put(Key, 1),
    One = persistent_term:get(Key),
    Default = persistent_term:get(Key, none),
    Found = lists:keyfind(Key, 1, persistent_term:get()),
    Erased = persistent_term:erase(Key),
    Gone = persistent_term:get(Key, none),
    Again = persistent_term:erase(Key),
    [Missing, Badarg, Put, Same, One, Default, Found, Erased, Gone, Again].

%% A child with a persistent_term capability of no operation tries a put of
%% a new key and of a changed value. A denied put stores nothing.
persistent_term_kept() ->
    Key = {?MODULE, kept},
    New = restricted(persistent_term, fun() -> persistent_term:put(Key, 1) end),
    Absent = persistent_term:get(Key, none),
    ok = persistent_term:put(Key, 1),
    Change = restricted(persistent_term, fun() -> persistent_term:put(Key, 2) end),
    Changed = persistent_term:get(Key),
    Erase = restricted(persistent_term, fun() -> persistent_term:erase(Key) end),
    Kept = persistent_term:get(Key, none),
    [{New, Absent}, {Change, Changed}, {Erase, Kept}].

%% Permitted terminal queries and setup. The results depend on the standard
%% streams and on TERM, so the test compares them with an ERTS run that has
%% the same streams and environment. The probe does not call setlocale/1,
%% which changes the locale of the OS process. ERTS needs the load of the
%% NIF library, which can give a reload error.
terminal_permitted() ->
    _ = prim_tty:load("prim_tty"),
    {ok, TTY} = prim_tty:tty_create(stdout),
    [
        prim_tty:isatty(stdin),
        prim_tty:isatty(stdout),
        reference(TTY),
        prim_tty:setupterm_nif(),
        prim_tty:tigetnum_nif("co\0"),
        prim_tty:tigetflag_nif("xn\0")
    ].

%% The fd and code checks come before the terminal capability check.
terminal_badargs() ->
    _ = prim_tty:load("prim_tty"),
    [
        badarg(fun() -> prim_tty:isatty(foo) end),
        badarg(fun() -> prim_tty:tty_create(stdin) end),
        badarg(fun() -> prim_tty:tty_create(foo) end),
        badarg(fun() -> prim_tty:tigetnum_nif(foo) end),
        badarg(fun() -> prim_tty:tigetflag_nif(foo) end)
    ].

%% A permitted port_info/1 of an open port. The id and the OS pid change
%% from run to run, so the probe gives only their keys.
inspect_permitted() ->
    Port = cat(),
    Info = erlang:port_info(Port),
    Self = self(),
    [
        [Key || {Key, _} <- Info],
        lists:keyfind(name, 1, Info),
        lists:keyfind(links, 1, Info) =:= {links, [Self]},
        lists:keyfind(connected, 1, Info) =:= {connected, Self},
        lists:keyfind(input, 1, Info),
        lists:keyfind(output, 1, Info)
    ].

%% A closed port and an atom give undefined with no check.
inspect_closed() ->
    Port = cat(),
    true = erlang:port_close(Port),
    [erlang:port_info(Port), erlang:port_info(effect_probe_nobody)].

%% The `executables` constraint of `port` compares the program name of the
%% port for port_info/1, also for a port of another process.
inspect_names() ->
    Port = cat(),
    Info = fun() -> erlang:port_info(Port) end,
    [
        in_child(#{port => #{executables => ["/bin/cat"]}, process => #{}}, Info),
        in_child(#{port => #{executables => ["/bin/sh"]}, process => #{}}, Info)
    ].

%% One call of each code entry. `erlang:finish_loading/1` checks the `code`
%% capability with `load`, and `erlang:module_loaded/1` and
%% `erlang:get_module_info/2` check it with `inspect` for the module name.
module_loaded() -> attempt(fun() -> erlang:module_loaded(?MODULE) end).

loaded() -> attempt(fun() -> erlang:loaded() end).

get_module_info() -> attempt(fun() -> erlang:get_module_info(?MODULE, module) end).

%% `erlang:get_module_info/1` checks the `code` capability with `inspect`
%% for the module name before it builds the pair list of `module_info/0`.
get_module_info_1() -> attempt(fun() -> erlang:get_module_info(?MODULE) end).

finish_loading() -> attempt(fun() -> erlang:finish_loading([]) end).

%% One call of each ambient and later entry, which has no capability check. A
%% process with no capability gets the same result as on ERTS. Each value that
%% depends on the host or on the run is reduced to a stable sign.
ambient() ->
    Self = self(),
    Name = effect_probe_ambient,
    Table = ets:new(Name, [named_table]),
    Selected = ets:new(effect_probe_selected, []),
    true = ets:insert(Selected, {key, value}),
    Array = erts_internal:atomics_new(1, 1),
    [
        erlang:unlink(Self),
        erlang:demonitor(make_ref()),
        erlang:demonitor(make_ref(), [info]),
        erlang:register(Name, Self),
        erlang:whereis(Name) =:= Self,
        erlang:unregister(Name),
        integer(erlang:monotonic_time()),
        integer(erlang:monotonic_time(nanosecond)),
        integer(os:system_time(second)),
        integer(erlang:system_time()),
        integer(erlang:system_time(second)),
        date_time(erlang:universaltime_to_localtime({{2026, 10, 11}, {12, 0, 0}})),
        date_time(erlang:localtime_to_universaltime({{2026, 10, 11}, {12, 0, 0}}, undefined)),
        element(1, erlang:system_info(os_type)),
        erlang:node(),
        erlang:node(Self),
        erlang:nodes(),
        erlang:nodes(this),
        length(erlang:nodes(this, #{})),
        erlang:function_exported(?MODULE, ambient, 0),
        attempt(fun() -> records:get_definition(?MODULE, none) end),
        erlang:fun_info(fun(X) -> X end, arity),
        element(3, erlang:fun_info_mfa(fun(X) -> X end)),
        erts_internal:prepare_loading(effect_probe_missing, <<"FOR1">>),
        attempt(fun() -> erlang:finish_after_on_load(effect_probe_missing, true) end),
        erlang:cancel_timer(make_ref(), []),
        erlang:cancel_timer(make_ref()),
        is_list(erlang:processes()),
        erlang:list_to_atom("effect_probe_ambient_atom"),
        erlang:binary_to_atom(<<"effect_probe_ambient_binary">>, utf8),
        erlang:binary_to_existing_atom(<<"effect_probe_ambient_binary">>, utf8),
        erlang:list_to_existing_atom("effect_probe_ambient_atom"),
        element(1, re:compile("a", [])),
        begin
            P = binary:compile_pattern(<<"a">>),
            [element(1, P), binary:match(<<"xax">>, P)]
        end,
        element(1, binary:compile_pattern([<<"a">>, <<"b">>])),
        element(1, re:import(element(2, re:compile("a", [export])))),
        Table,
        ets:insert(Name, {key, value}),
        ets:insert_new(Name, {key, value}),
        ets:lookup(Name, key),
        ets:lookup_element(Name, key, 2, none),
        ets:lookup_element(Name, key, 2),
        ets:member(Name, key),
        ets:safe_fixtable(Name, true),
        ets:take(Name, key),
        ets:delete(Name, key),
        ets:delete_object(Name, {key, value}),
        ets:info(Name, type),
        ets:insert(Name, {key, value}),
        length(ets:info(Name)),
        case ets:next(Name, key) of
            '$end_of_table' -> eot;
            Next -> Next
        end,
        case ets:prev(Name, key) of
            '$end_of_table' -> eot;
            Previous -> Previous
        end,
        case ets:first(Name) of
            key -> first;
            First -> First
        end,
        case ets:first_lookup(Name) of
            {key, [{key, value}]} -> first_lookup;
            Lookup -> Lookup
        end,
        case ets:last(Name) of
            key -> last;
            Last -> Last
        end,
        case ets:last_lookup(Name) of
            {key, [{key, value}]} -> last_lookup;
            LastLookup -> LastLookup
        end,
        case ets:next_lookup(Name, key) of
            '$end_of_table' -> eot;
            NextLookup -> NextLookup
        end,
        case ets:prev_lookup(Name, key) of
            '$end_of_table' -> eot;
            PrevLookup -> PrevLookup
        end,
        ets:match_object(Name, {'_', '_'}),
        ets:select(Name, [{{'_', '_'}, [], ['$_']}]),
        begin
            {_, C} = ets:select(Name, [{{'_', '_'}, [], ['$_']}], 1),
            case ets:select(C) of
                '$end_of_table' -> eot;
                {Matches, '$end_of_table'} -> {Matches, eot};
                Other -> Other
            end
        end,
        ets:update_element(Selected, key, {2, updated}),
        ets:update_element(Selected, missing, {2, updated}, {missing, 0}),
        ets:internal_select_delete(Selected, [{{'_', '_'}, [], [false]}]),
        ets:internal_delete_all(Selected, undefined),
        reference(ets:whereis(Name)),
        ets:delete(ets:new(effect_probe_deleted, [])),
        atom(file:native_name_encoding()),
        byte_size(prim_file:internal_name2native("a")),
        element(1, prim_file:read_file_info("/")),
        length(prim_file:internal_native2name(<<"a">>)),
        element(1, prim_file:read_file("/")),
        element(1, prim_file:list_dir("/")),
        element(1, prim_file:read_link("/")),
        element(1, prim_file:get_cwd()),
        element(1, prim_file:open("/", [read])),
        null_write(),
        null_read(),
        buffer(),
        erts_internal:processes_next(1048576),
        erts_internal:is_system_process(Self),
        attempt(fun() -> erts_internal:purge_module(effect_probe_missing, complete) end),
        erts_internal:request_system_task(Self, normal, {check_process_code, Name, lists}),
        receive {check_process_code, Name, Refers} -> Refers end,
        reference(Array),
        atomics:add(Array, 1, 1),
        atomics:get(Array, 1),
        atomics:put(Array, 1, 7),
        maps:get(memory, atomics:info(Array)),
        atomics:add_get(Array, 1, 1),
        atomics:exchange(Array, 1, 3),
        atomics:compare_exchange(Array, 1, 3, 4),
        %% The default arity clock BIFs and the host identifiers, each
        %% reduced to a stable sign by the helpers below. The sibling task
        %% #754 already calls `erlang:monotonic_time/0` and
        %% `erlang:system_time/0` above.
        integer(erlang:time_offset()),
        integer(erlang:time_offset(second)),
        timestamp(erlang:timestamp()),
        date(erlang:date()),
        time(erlang:time()),
        date_time(erlang:localtime()),
        date_time(erlang:universaltime()),
        timestamp(erlang:now()),
        integer(os:system_time()),
        timestamp(os:timestamp()),
        integer(os:perf_counter()),
        integer(list_to_integer(os:getpid())),
        prim_file:altname("/"),
        element(1, prim_file:delete("/pbeam-missing")),
        element(1, prim_file:del_dir("/pbeam-missing")),
        element(1, prim_file:make_dir("/pbeam-missing/dir")),
        element(1, prim_file:make_link("/pbeam-missing", "/pbeam-missing2")),
        element(1, prim_file:make_symlink("target", "/pbeam-missing/link")),
        element(1, prim_file:rename("/pbeam-missing", "/pbeam-missing2")),
        element(1, prim_file:set_cwd("/pbeam-missing")),
        set_owner_probe(),
        set_permissions_probe(),
        set_time_probe(),
        is_binary(erlang:term_to_binary(1)),
        is_binary(erlang:term_to_binary(1, [])),
        erlang:binary_to_term(erlang:term_to_binary(1)) =:= 1,
        erlang:binary_to_term(erlang:term_to_binary(1), []) =:= 1,
        erlang:external_size(<<1, 2, 3>>),
        erlang:external_size(<<1, 2, 3>>, []),
        is_list(erlang:term_to_iovec(1)),
        is_list(erlang:term_to_iovec(1, [])),
        erlang:read_timer(make_ref()),
        is_reference(erlang:alias([explicit_unalias])),
        alias_unalias(),
        is_list(erlang:registered()),
        is_list(erlang:ports()),
        erlang:display_string(stdout, <<>>),
        erlang:display(effect_probe_ambient_display),
        attempt(fun() -> erlang:has_prepared_code_on_load(make_ref()) end),
        is_list(prim_socket:supports()),
        prim_socket:supports(other),
        element(1, prim_net:gethostname()),
        maps:get(netns, prim_net:info()),
        prim_net:debug(false)
    ].

%% Each metadata probe makes a temporary file, so `write_file_info/3`
%% reaches `set_owner_nif/3`, `set_permissions_nif/2`, and `set_time_nif/4`.
set_owner_probe() ->
    metadata_probe("owner", fun(Path) ->
        prim_file:write_file_info(Path, #file_info{uid = undefined, gid = undefined, atime = 0, mtime = 0, ctime = 0}, [{time, posix}])
    end).

set_permissions_probe() ->
    metadata_probe("permissions", fun(Path) ->
        prim_file:write_file_info(Path, #file_info{mode = 8#644, atime = 0, mtime = 0, ctime = 0}, [{time, posix}])
    end).

set_time_probe() ->
    metadata_probe("time", fun(Path) ->
        prim_file:write_file_info(Path, #file_info{atime = 0, mtime = 0, ctime = 0}, [{time, posix}])
    end).

metadata_probe(Name, Operation) ->
    Path = "/tmp/pbeam-effect-" ++ Name ++ "-" ++ os:getpid(),
    ok = prim_file:write_file(Path, <<>>),
    Result = Operation(Path),
    ok = prim_file:delete(Path),
    Result.

%% Make an explicit_unalias alias and end it.
alias_unalias() ->
    Alias = erlang:alias([explicit_unalias]),
    true = erlang:unalias(Alias),
    true.

%% Open /dev/null in write mode, write, move the position, and close it
%% with prim_file. A write-only file keeps no prim_buffer read buffer
%% (prim_file.erl:512-527).
null_write() ->
    {ok, Fd} = prim_file:open("/dev/null", [write]),
    [prim_file:write(Fd, <<>>), prim_file:position(Fd, 0), prim_file:close(Fd)].

%% Open /dev/null in read mode, read its end, and close it with prim_file.
%% A file with a read mode keeps a read buffer of prim_buffer, and
%% prim_file:read/2 runs prim_file:read_nif/2 (prim_file.erl:162-204,
%% 516-517).
null_read() ->
    {ok, Fd} = prim_file:open("/dev/null", [read]),
    [prim_file:read(Fd, 1), prim_file:close(Fd)].

%% Each NIF of prim_buffer, the read buffer of a file with a read mode
%% (prim_file.erl:516-517): new/0, size/1, write/2, find_byte_index/2,
%% read/2 with copying_read/2 and peek_head/1, wipe/1 with skip/2, and the
%% lock of try_lock/1 and unlock/1.
buffer() ->
    Small = prim_buffer:new(),
    prim_buffer:write(Small, [<<"0123456789\n">>]),
    Large = prim_buffer:new(),
    prim_buffer:write(Large, [<<"xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx">>]),
    [
        prim_buffer:size(Small),
        prim_buffer:read(Small, 2),
        prim_buffer:find_byte_index(Small, $\n),
        prim_buffer:size(Small),
        prim_buffer:wipe(Small),
        prim_buffer:size(Small),
        prim_buffer:read(Large, 100),
        prim_buffer:try_lock(Large),
        prim_buffer:unlock(Large)
    ].

%% Guard tests, so the compiler does not call a BIF.
integer(Term) when is_integer(Term) -> integer;
integer(_) -> other.

atom(Term) when is_atom(Term) -> atom;
atom(_) -> other.

date_time({{_, _, _}, {_, _, _}}) -> date_time;
date_time(_) -> other.

timestamp({Mega, Sec, Micro}) when is_integer(Mega), is_integer(Sec), is_integer(Micro) ->
    timestamp;
timestamp(_) -> other.

date({Year, Month, Day}) when is_integer(Year), is_integer(Month), is_integer(Day) -> date;
date(_) -> other.

time({Hour, Minute, Second}) when is_integer(Hour), is_integer(Minute), is_integer(Second) ->
    time;
time(_) -> other.

%% Permitted process calls that give the same result on ERTS.
process_permitted() ->
    [link_kept(), monitor_kept(), exit_kept(), exit_self(), send_kept(), timer_kept()].

%% The argument checks come before the process capability check.
process_badargs() ->
    [
        badarg(fun() -> erlang:monitor(process, id(1)) end),
        badarg(fun() -> erlang:monitor(id(foo), self()) end),
        badarg(fun() -> erlang:monitor(process, self(), id([foo])) end),
        badarg(fun() -> erlang:exit(id(1), normal) end),
        badarg(fun() -> erlang:send(id(1), ping) end),
        badarg(fun() -> erlang:send(self(), ping, id([foo])) end),
        badarg(fun() -> erlang:start_timer(-1, self(), ping) end),
        badarg(fun() -> erlang:start_timer(0, id(1), ping) end),
        badarg(fun() -> erlang:link(id(1)) end)
    ].

%% The pid and item checks come before the port capability check.
port_badargs() ->
    Port = cat(),
    [
        badarg(fun() -> erlang:port_connect(Port, id(self)) end),
        badarg(fun() -> erlang:port_info(Port, id(bad_item)) end),
        badarg(fun() -> erlang:port_info(Port, id("connected")) end)
    ].

%% Each probe below gives the result of the call and a sign of its effect.
%% A denied call has no effect.

%% A link makes the exit of the child an EXIT message. The link exit signal
%% comes before the DOWN message of the same exit.
link_kept() ->
    process_flag(trap_exit, true),
    {Child, Monitor} = erlang:spawn_monitor(?MODULE, stop, []),
    Link = attempt(fun() -> erlang:link(Child) end),
    Child ! stop,
    receive
        {'DOWN', Monitor, process, Child, boom} -> ok
    end,
    Exit =
        receive
            {'EXIT', Child, boom} -> linked
        after 0 -> unlinked
        end,
    process_flag(trap_exit, false),
    {Link, Exit}.

%% A monitor of a name that no process has gives a noproc DOWN message at
%% once.
monitor_kept() ->
    [
        {attempt(fun() -> erlang:monitor(process, effect_probe_nobody) end), down()},
        {attempt(fun() -> erlang:monitor(process, effect_probe_nobody, []) end), down()}
    ].

%% A kill gives the DOWN reason killed. Else the child exits with boom.
exit_kept() ->
    {Child, Monitor} = erlang:spawn_monitor(?MODULE, stop, []),
    Exit = attempt(fun() -> erlang:exit(Child, kill) end),
    Child ! stop,
    receive
        {'DOWN', Monitor, process, Child, Reason} -> {Exit, Reason}
    end.

%% An exit signal to the caller, which traps exits, gives an EXIT message.
exit_self() ->
    process_flag(trap_exit, true),
    Self = self(),
    Exit = attempt(fun() -> erlang:exit(Self, normal) end),
    Message =
        receive
            {'EXIT', Self, normal} -> exit
        after 0 -> none
        end,
    process_flag(trap_exit, false),
    {Exit, Message}.

%% The send instruction, erlang:send/2, and erlang:send/3. A send to a name
%% that no process has raises badarg after the check.
send_kept() ->
    Self = self(),
    [
        {attempt(fun() -> Self ! ping end), ping()},
        {attempt(fun() -> erlang:send(Self, ping) end), ping()},
        {attempt(fun() -> erlang:send(Self, ping, []) end), ping()},
        attempt(fun() -> effect_probe_nobody ! ping end)
    ].

timer_kept() ->
    Timer = attempt(fun() -> erlang:start_timer(0, self(), ping) end),
    Fired =
        receive
            {timeout, _, ping} -> fired
        after 100 -> none
        end,
    {Timer, Fired}.

%% Permitted process inspection and group leader calls that give the same
%% result on ERTS, for a live process and for a process that exited. Each
%% reply comes before the next call.
process_inspect_permitted() ->
    Self = self(),
    Target = erlang:spawn(?MODULE, leader, []),
    [Ref1, Ref2, Ref3, Ref4] = [make_ref() || _ <- [1, 2, 3, 4]],
    Live = [
        erlang:process_info(Target, registered_name),
        erlang:process_info(Target, message_queue_len),
        erlang:is_process_alive(Target),
        erts_internal:is_process_alive(Target, Ref1),
        reply(Ref1),
        erlang:group_leader(Self, Target),
        erts_internal:group_leader(Self, Target, Ref2),
        reply(Ref2)
    ],
    Monitor = erlang:monitor(process, Target),
    Target ! {leader, Self},
    Leader =
        receive
            {leader, Pid} -> Pid =:= Self
        end,
    receive
        {'DOWN', Monitor, process, Target, normal} -> ok
    end,
    Dead = [
        erlang:process_info(Target, registered_name),
        erlang:is_process_alive(Target),
        erts_internal:is_process_alive(Target, Ref3),
        reply(Ref3),
        badarg(fun() -> erlang:group_leader(Self, Target) end),
        erts_internal:group_leader(Self, Target, Ref4),
        reply(Ref4)
    ],
    [Live, Leader, Dead].

%% The argument checks come before the process capability check.
%% erts_internal:group_leader/2 gives the atom badarg with no exception.
process_inspect_badargs() ->
    Other = erlang:spawn(?MODULE, stop, []),
    Alias = erlang:monitor(process, Other, [{alias, demonitor}]),
    [
        badarg(fun() -> erlang:process_info(id(1), registered_name) end),
        badarg(fun() -> erlang:process_info(Other, id(foo)) end),
        badarg(fun() -> erlang:is_process_alive(id(1)) end),
        badarg(fun() -> erts_internal:is_process_alive(Other, id(1)) end),
        badarg(fun() -> erts_internal:is_process_alive(Other, Alias) end),
        erts_internal:group_leader(id(1), Other),
        erts_internal:group_leader(self(), id(1)),
        badarg(fun() -> erts_internal:group_leader(id(1), Other, make_ref()) end),
        badarg(fun() -> erts_internal:group_leader(self(), Other, id(1)) end)
    ].

%% Calls for the caller, which have no check.
process_inspect_self() ->
    Self = self(),
    Leader = group_leader(),
    [Ref1, Ref2] = [make_ref(), make_ref()],
    [
        erlang:process_info(Self, registered_name),
        erlang:is_process_alive(Self),
        erts_internal:is_process_alive(Self, Ref1),
        reply(Ref1),
        erlang:group_leader(Leader, Self),
        erts_internal:group_leader(Leader, Self, Ref2),
        reply(Ref2)
    ].

%% A child with no inspect asks about a process that exited. A denied call
%% does not tell that the process exited, and a denied is-alive request
%% sends no reply.
inspect_kept() ->
    Dead = dead(),
    uninspecting(fun() ->
        Ref = make_ref(),
        [
            attempt(fun() -> erlang:process_info(Dead, registered_name) end),
            attempt(fun() -> erlang:is_process_alive(Dead) end),
            {attempt(fun() -> erts_internal:is_process_alive(Dead, Ref) end), reply(Ref, 0)}
        ]
    end).

%% A child with no group_leader tries to make itself the group leader of a
%% process. A denied call does not change the group leader.
group_leader_kept() ->
    Target = erlang:spawn(?MODULE, leader, []),
    Original = group_leader(),
    Result = uninspecting(fun() ->
        Self = self(),
        [
            attempt(fun() -> erts_internal:group_leader(Self, Target) end),
            attempt(fun() -> erts_internal:group_leader(Self, Target, make_ref()) end)
        ]
    end),
    Target ! {leader, self()},
    receive
        {leader, Original} -> {Result, same};
        {leader, _} -> {Result, changed}
    end.

%% Run Call in a child whose process capability has no inspect and no
%% group_leader, and give its value.
uninspecting(Call) ->
    Operations = [spawn, link, monitor, exit_other, send],
    erlang:spawn_opt(?MODULE, value_child, [self(), Call], [
        {capabilities, #{process => #{operations => Operations}}}
    ]),
    receive
        {child, Result} -> Result
    end.

value_child(Parent, Call) -> Parent ! {child, Call()}.

%% Give the group leader to the first process that asks, then exit.
leader() ->
    receive
        {leader, From} -> From ! {leader, group_leader()}
    end.

dead() ->
    {Pid, Monitor} = erlang:spawn_monitor(?MODULE, done, []),
    receive
        {'DOWN', Monitor, process, Pid, normal} -> Pid
    end.

reply(Ref) -> reply(Ref, 1000).

reply(Ref, Timeout) ->
    receive
        {Ref, Result} -> Result
    after Timeout -> none
    end.

down() ->
    receive
        {'DOWN', _, process, {effect_probe_nobody, _}, noproc} -> down
    after 0 -> none
    end.

ping() ->
    receive
        ping -> got
    after 0 -> none
    end.

stop() ->
    receive
        stop -> exit(boom)
    end.

id(Term) -> Term.

%% A guard test, so the compiler does not call erlang:is_reference/1.
reference(Term) when is_reference(Term) -> reference;
reference(_) -> other.

%% Run Call in a child whose capability of Kind permits no operation.
restricted(Kind, Call) ->
    in_child(#{Kind => #{operations => []}, process => #{}}, Call).

%% Run Call in a child with the capabilities option Capabilities.
in_child(Capabilities, Call) ->
    erlang:spawn_opt(?MODULE, child, [self(), Call], [{capabilities, Capabilities}]),
    receive
        {child, Result} -> Result
    end.

child(Parent, Call) -> Parent ! {child, attempt(Call)}.

%% The `names` constraint of `env` compares the native bytes of the name,
%% also of a nested list.
names() ->
    [
        attempt(fun() -> os:getenv("PBEAM_EFFECT_PROBE") end),
        attempt(fun() -> os:getenv(["PBEAM_EFFECT", ["_PROBE"]]) end),
        attempt(fun() -> os:getenv("PBEAM_EFFECT_PROB") end),
        attempt(fun() -> os:getenv("PBEAM_EFFECT_PROBE_") end),
        attempt(fun() -> os:getenv("HOME") end)
    ].

%% The `executables` constraint of `port` compares the program name as the
%% caller gives it, for open_port/2, port_command/2, and port_close/1.
executables() ->
    Port = cat(),
    Command = attempt(fun() -> erlang:port_command(Port, <<"x">>) end),
    <<"x">> = echo(Port),
    [
        Command,
        attempt(fun() -> erlang:port_close(Port) end),
        attempt(fun() -> erlang:open_port({spawn_executable, <<"/bin/cat">>}, [binary]) end),
        attempt(fun() -> erlang:open_port({spawn_executable, "/bin/sh"}, [binary]) end),
        attempt(fun() -> erlang:open_port({spawn_executable, "/bin/../bin/cat"}, [binary]) end),
        attempt(fun() -> erlang:open_port({fd, 1, 2}, [binary]) end)
    ].

%% The `signals` constraint of `signal`. SIG_DFL is the start disposition
%% of SIGUSR2.
signals() ->
    [
        attempt(fun() -> os:set_signal(sigusr2, default) end),
        attempt(fun() -> os:set_signal(sighup, default) end)
    ].

%% The `modules` constraint of `code` compares the module atom of the
%% `inspect` entries, and each module of the list of the `load` entry. An
%% empty list has no module name, so it only needs the `load` operation.
%% `erlang:loaded/0` needs `inspect` for each loaded module.
code_names() ->
    [
        attempt(fun() -> erlang:loaded() end),
        attempt(fun() -> erlang:module_loaded(?MODULE) end),
        attempt(fun() -> erlang:module_loaded(lists) end),
        attempt(fun() -> erlang:get_module_info(?MODULE, module) end),
        attempt(fun() -> erlang:get_module_info(lists, module) end),
        attempt(fun() -> erlang:finish_loading([]) end)
    ].

%% The argument checks of `erlang:get_module_info/2` come before its
%% capability check, as they do in `erlang:process_info/2`: a key that is
%% not an atom raises `badarg`, also in a process with no capability.
code_badarg() ->
    attempt(fun() -> erlang:get_module_info(?MODULE, 1) end).

%% A denied port_close/1 keeps the port open.
close_kept() ->
    Port = cat(),
    Close = attempt(fun() -> erlang:port_close(Port) end),
    case erlang:port_info(Port) of
        undefined -> {Close, closed};
        Info when is_list(Info) -> {Close, open}
    end.

cat() -> erlang:open_port({spawn_executable, "/bin/cat"}, [binary]).

%% Wait for the echo of cat, so that a close does not stop its write.
echo(Port) ->
    receive
        {Port, {data, Data}} -> Data
    end.

attempt(Call) ->
    try Call() of
        _ -> ok
    catch
        error:capability_denied -> denied;
        Class:Reason -> {Class, Reason}
    end.

badarg(Call) ->
    try Call() of
        Result -> {ok, Result}
    catch
        error:badarg -> badarg
    end.

done() -> ok.
