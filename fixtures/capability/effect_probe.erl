-module(effect_probe).
-export([
    spawn/0,
    spawn_link/0,
    spawn_opt/0,
    spawn_opt_link/0,
    spawn_opt_monitor/0,
    open_port/0,
    port_command/0,
    port_close/0,
    getenv/0,
    handle_signal/0,
    ignore_signal/0,
    default_signal/0,
    write_nif/0,
    read_nif/0,
    tty_select/0,
    tty_init/0,
    tty_window_size/0,
    halt_run/0,
    system_flag/0,
    load_nif/0,
    link/0,
    monitor2/0,
    monitor3/0,
    exit_other/0,
    send_instruction/0,
    send2/0,
    send3/0,
    start_timer/0,
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
    child/2,
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

getenv() -> attempt(fun() -> os:getenv("PBEAM_EFFECT_PROBE") end).

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

%% `!` compiles to the send instruction, and erlang:send/2 to a call of the
%% BIF.
send_instruction() -> attempt(fun() -> self() ! ping end).

send2() -> attempt(fun() -> erlang:send(self(), ping) end).

send3() -> attempt(fun() -> erlang:send(self(), ping, []) end).

start_timer() -> attempt(fun() -> erlang:start_timer(0, self(), ping) end).

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
    Denied = restricted(fun() -> erlang:system_flag(system_logger, undefined) end),
    {Denied, erlang:system_flag(system_logger, logger)}.

load_nif_kept() ->
    Denied = restricted(fun() -> prim_tty:load("prim_tty") end),
    {Denied, prim_tty:load("prim_tty")}.

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
        badarg(fun() -> erlang:start_timer(0, id(1), ping) end)
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

restricted(Call) ->
    Capabilities = #{runtime => #{operations => []}, process => #{}},
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
        attempt(fun() -> erlang:open_port({spawn_executable, "/bin/../bin/cat"}, [binary]) end)
    ].

%% The `signals` constraint of `signal`. SIG_DFL is the start disposition
%% of SIGUSR2.
signals() ->
    [
        attempt(fun() -> os:set_signal(sigusr2, default) end),
        attempt(fun() -> os:set_signal(sighup, default) end)
    ].

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
