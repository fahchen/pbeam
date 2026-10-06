-module(cooked_probe).
-export([main/0, read/0, pbeam_main/0, pbeam_read/0, own/1, init/1, reader/1]).

main() ->
    run(init).

read() ->
    run(reader).

%% PBeam has no kernel boot, so pbeam_main/0 and pbeam_read/0 first make
%% ac_tab with the options of application_controller.erl:544-545 and start
%% erl_signal_server with erl_signal_handler, as kernel.erl:41,141-146 do.
pbeam_main() ->
    boot(),
    main().

pbeam_read() ->
    boot(),
    read().

boot() ->
    Parent = self(),
    spawn(?MODULE, own, [Parent]),
    receive started -> ok end.

own(Parent) ->
    ac_tab = ets:new(ac_tab, [set, public, named_table, {read_concurrency, true}]),
    {ok, _} = gen_event:start_link({local, erl_signal_server}),
    ok = erl_signal_handler:start(),
    Parent ! started,
    receive stop -> ok end.

run(Function) ->
    Pid = spawn(?MODULE, Function, [self()]),
    receive {Pid, Result} -> Result end.

init(Parent) ->
    true = register(cooked_probe_tty, self()),
    State = prim_tty:init(#{input => cooked, output => raw}),
    ReaderRef = maps:get(read, prim_tty:handles(State)),
    erl_signal_server ! {notify, sigwinch},
    Signal = receive {ReaderRef, {signal, S}} -> S end,
    Parent ! {self(), [prim_tty:unicode(State), prim_tty:output_mode(State),
                       prim_tty:is_reader(State, whereis(cooked_probe_tty_reader)),
                       prim_tty:is_writer(State, whereis(cooked_probe_tty_writer)),
                       prim_tty:window_size(State), Signal,
                       gen_event:which_handlers(erl_signal_server)]}.

reader(Parent) ->
    true = register(cooked_probe_read, self()),
    State = prim_tty:init(#{input => cooked, output => raw}),
    ReaderRef = maps:get(read, prim_tty:handles(State)),
    ok = prim_tty:read(State),
    Parent ! {self(), receive {ReaderRef, Data} -> Data end}.
