-module(kernel_boot_probe).
-behaviour(application).
-behaviour(supervisor).
-export([main/0, main/1, start/2, stop/1, init/1]).

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% main -kernel logger_level warning` runs main/0 in the boot process of init after the boot script
%% starts kernel and stdlib (init.erl:1539-1571). main/0 displays the
%% kernel processes, the started applications, and the results of
%% application:start/1, ensure_all_started/1,2, and stop/1 for the
%% applications of kernel_boot_app.app and kernel_boot_dep.app in
%% fixtures/boot. It then calls init:stop/0, and main/1 calls
%% init:stop/1 with its status. init stops application_controller, which
%% stops each running application (application_controller.erl:1263-1300),
%% so stop/1 of kernel_boot_app displays its state, and init halts with
%% the status (init.erl:924-948). `mise run fixture-beam` compiles this
%% file.

main() ->
    report(),
    init:stop().

main([Status]) ->
    report(),
    init:stop(list_to_integer(atom_to_list(Status))).

report() ->
    Controller = whereis(application_controller),
    Kernel = application:which_applications(),
    StartKernel = application:start(kernel),
    StartMissing = application:start(kernel_boot_missing),
    EnsureKernel = application:ensure_all_started(kernel),
    EnsureDep = application:ensure_all_started(kernel_boot_dep),
    Running = application:which_applications(),
    StartAgain = application:start(kernel_boot_app),
    StopDep = application:stop(kernel_boot_dep),
    StopDepAgain = application:stop(kernel_boot_dep),
    EnsureTemporary = application:ensure_all_started([kernel_boot_dep], temporary),
    EnsureMissing = application:ensure_all_started(kernel_boot_missing),
    StopBadArg = try init:stop(stop) catch Class:Reason -> {Class, Reason} end,
    %% init kills each process that is not a system process at the stop
    %% (init.erl:1082-1093); init is a system process (erl_init.c:355-399).
    System = [erts_internal:is_system_process(Pid) || Pid <- [whereis(init), Controller, self()]],
    SystemBadArg = try erts_internal:is_system_process(init) catch Class2:Reason2 -> {Class2, Reason2} end,
    erlang:display({controller, process_info(Controller, registered_name),
                    controller_linked_to_init,
                    lists:member(Controller, element(2, process_info(whereis(init), links))),
                    kernel, Kernel,
                    start_kernel, StartKernel,
                    start_missing, StartMissing,
                    ensure_kernel, EnsureKernel,
                    ensure_dep, EnsureDep,
                    running, Running,
                    start_again, StartAgain,
                    stop_dep, StopDep,
                    stop_dep_again, StopDepAgain,
                    ensure_temporary, EnsureTemporary,
                    ensure_missing, EnsureMissing,
                    stop_badarg, StopBadArg,
                    system, System,
                    system_badarg, SystemBadArg,
                    status, init:get_status()}).

start(normal, []) ->
    supervisor:start_link({local, kernel_boot_sup}, ?MODULE, []).

stop(State) ->
    erlang:display({stop, kernel_boot_app, State, whereis(kernel_boot_sup),
                    init:get_status()}),
    ok.

init([]) ->
    {ok, {#{}, []}}.
