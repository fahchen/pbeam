-module(pbeam_boot).
-export([start/2, kernel/1]).

%% Run the boot and then Module:Function(), as the init boot process runs
%% the boot script and then -eval (init.erl:1192-1217). The boot process
%% traps exits (init.erl:1197) and sets the registered user process as the
%% group leader before it starts -eval (init.erl:1539-1546).
start(Module, Function) ->
    process_flag(trap_exit, true),
    Kernel = spawn(?MODULE, kernel, [self()]),
    receive {Kernel, started} -> ok end,
    group_leader(whereis(user), self()),
    Module:Function().

%% Do the work of application_controller and kernel_sup. Make ac_tab with
%% the options of application_controller.erl:544-545. Start the children in
%% the order of kernel.erl:124-153, and then erl_signal_handler, as
%% kernel:start/2 does (kernel.erl:39-41). user_sup:start/0 reads the flags
%% from init (user_sup.erl:41-47), so give it the flags of erl -noshell
%% directly. Trap exits, as a supervisor does (supervisor.erl:950), and stay
%% alive as the link parent of the servers.
kernel(Boot) ->
    process_flag(trap_exit, true),
    _ = ets:new(ac_tab, [set, public, named_table, {read_concurrency, true}]),
    {ok, _} = standard_error:start_link(),
    {ok, _} = gen_event:start_link({local, erl_signal_server}),
    {ok, _} = supervisor_bridge:start_link(user_sup, [{noshell, []}]),
    ok = erl_signal_handler:start(),
    Boot ! {self(), started},
    receive after infinity -> ok end.
