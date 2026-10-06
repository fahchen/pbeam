-module(host_write_probe).
-export([main/0, pbeam_main/0, own/1, init/3]).

%% Write "Hello, world!\n" to stdout through a prim_tty writer, then write
%% the reply of that writer and a newline to stderr through a second writer.
main() ->
    Out = init(host_write_probe_out, #{input => disabled, output => raw}),
    Err = init(host_write_probe_err, #{input => disabled, output => raw, ofd => stderr}),
    Reply = write(Out, "Hello, world!\n"),
    write(Err, atom_to_list(Reply) ++ "\n").

%% PBeam has no application_controller, so pbeam_main/0 first makes ac_tab
%% with the options of application_controller.erl:544-545.
pbeam_main() ->
    Parent = self(),
    spawn(?MODULE, own, [Parent]),
    receive ac_tab -> main() end.

own(Parent) ->
    ac_tab = ets:new(ac_tab, [set, public, named_table, {read_concurrency, true}]),
    Parent ! ac_tab,
    receive stop -> ok end.

init(Name, Options) ->
    Pid = spawn(?MODULE, init, [self(), Name, Options]),
    receive {Pid, State} -> State end.

init(Parent, Name, Options) ->
    true = register(Name, self()),
    Parent ! {self(), prim_tty:init(Options)},
    receive stop -> ok end.

write(State, Chars) ->
    #{write := WriterRef} = prim_tty:handles(State),
    {ok, _Monitor} = prim_tty:write(State, Chars, self()),
    receive {WriterRef, Reply} -> Reply end.
