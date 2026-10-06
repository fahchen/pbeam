-module(tty_init_probe).
-export([main/0, pbeam_main/0, init/3, own/1]).

main() ->
    [prim_tty:isatty(stdin), prim_tty:isatty(stdout), prim_tty:isatty(stderr),
     init(tty_init_probe_raw, #{input => disabled, output => raw}),
     init(tty_init_probe_stderr, #{input => disabled, output => cooked, ofd => stderr}),
     init(tty_init_probe_stdin, #{input => disabled, ofd => stdin})].

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
    receive {Pid, Result} -> Result end.

init(Parent, Name, Options) ->
    true = register(Name, self()),
    Result = try prim_tty:init(Options) of
                 State ->
                     Writer = whereis(list_to_atom(atom_to_list(Name) ++ "_writer")),
                     [prim_tty:unicode(State), prim_tty:output_mode(State),
                      maps:get(read, prim_tty:handles(State)),
                      prim_tty:is_writer(State, Writer), prim_tty:window_size(State)]
             catch error:Reason -> Reason
             end,
    Parent ! {self(), Result}.
