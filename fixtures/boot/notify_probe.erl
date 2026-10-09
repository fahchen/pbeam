-module(notify_probe).
-behaviour(gen_event).
-export([notify/0, busy/0, wait/0, init/1, handle_event/2, handle_call/2]).

%% Handle sighup and sigusr2, and add a handler to erl_signal_server that
%% sends each event to the probe. Write "ready", then write each of the two
%% next events. ERTS sends {notify, Signal} to erl_signal_server for each
%% handled signal (sys.c:589-607, 1163-1176), and gen_event gives Signal to
%% each handler. The default erl_signal_handler ignores both signals
%% (erl_signal_handler.erl:49-50).
notify() ->
    ok = os:set_signal(sighup, handle),
    ok = os:set_signal(sigusr2, handle),
    ok = gen_event:add_handler(erl_signal_server, ?MODULE, self()),
    io:put_chars("ready\n"),
    [receive {signal, Signal} -> io:put_chars([atom_to_list(Signal), "\n"]) end || _ <- [1, 2]],
    ok.

%% As notify/0 for sighup, while a process that never waits can always
%% run. The signal must reach erl_signal_server although the scheduler is
%% never idle.
busy() ->
    ok = os:set_signal(sighup, handle),
    ok = gen_event:add_handler(erl_signal_server, ?MODULE, self()),
    spawn(fun Loop() -> Loop() end),
    io:put_chars("ready\n"),
    receive {signal, Signal} -> io:put_chars([atom_to_list(Signal), "\n"]) end.

init(Probe) ->
    {ok, Probe}.

handle_event(Signal, Probe) ->
    Probe ! {signal, Signal},
    {ok, Probe}.

handle_call(_Request, Probe) ->
    {ok, ok, Probe}.

%% Write "ready" and wait with no timer and no input select, so only a host
%% signal can end the run.
wait() ->
    io:put_chars("ready\n"),
    receive after infinity -> ok end.
