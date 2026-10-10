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
    permitted/0,
    names/0,
    executables/0,
    signals/0,
    close_kept/0,
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
