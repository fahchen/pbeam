-module(pbeam_boot).
-export([start/2, init/2, kernel/1]).

%% Run the boot and then Module:Function(), as the init boot process runs
%% start_clean.boot and then -eval (init.erl:1192-1217). The boot process
%% traps exits (init.erl:1197). It registers the init request process, and
%% starts the kernel processes logger and application_controller in the
%% order of start_clean.boot (init.erl:1348-1350). It then runs the kernel
%% part of kernel:start/2, and sets the registered user process as the
%% group leader before it starts -eval (init.erl:1539-1546). In a pbeam run
%% in boot mode (--boot), the process then enters the runtime: it drops the
%% boot authority and gets the capabilities that the policy gives
%% Module:Function/0. In each other run, pbeam_cap:enter_runtime/2 returns
%% ok and has no effect. ERTS has no pbeam_cap module.
start(Module, Function) ->
    process_flag(trap_exit, true),
    Init = spawn(?MODULE, init, [self(), [{noshell, []}]]),
    true = register(init, Init),
    ok = start_in_kernel(Init, logger, logger_server, start_link, []),
    ok = start_in_kernel(Init, application_controller,
                         application_controller, start, [kernel_app()]),
    Kernel = spawn(?MODULE, kernel, [self()]),
    receive {Kernel, started} -> ok end,
    Init ! {self(), kernel, Kernel},
    group_leader(whereis(user), self()),
    ok = pbeam_cap:enter_runtime(Module, Function),
    Module:Function().

%% Start a kernel process and give its link to init (init.erl:1519-1528).
start_in_kernel(Init, Server, Mod, Fun, Args) ->
    Res = apply(Mod, Fun, Args),
    Init ! {self(), started, {Server, Res}},
    receive
        {Init, ok, Pid} ->
            unlink(Pid),
            ok
    end.

%% The init request process. It traps exits (init.erl:588), links to each
%% kernel process that starts with {ok, Pid} (init.erl:768-772), and answers
%% get_arguments and {get_argument, Arg} (init.erl:816-819). For init:stop/0
%% it stops the kernel process and halts with status 0 (init.erl:536,
%% 788-789, 924-945). It keeps other messages in its mailbox.
init(Boot, Flags) ->
    process_flag(trap_exit, true),
    init_loop(Boot, Flags, undefined).

init_loop(Boot, Flags, Kernel) ->
    receive
        {Boot, started, {_Server, {ok, Pid}}} when is_pid(Pid) ->
            link(Pid),
            Boot ! {self(), ok, Pid},
            init_loop(Boot, Flags, Kernel);
        {Boot, kernel, Pid} ->
            init_loop(Boot, Flags, Pid);
        {From, get_arguments} ->
            From ! {init, Flags},
            init_loop(Boot, Flags, Kernel);
        {From, {get_argument, Arg}} ->
            From ! {init, get_argument(Arg, Flags)},
            init_loop(Boot, Flags, Kernel);
        {stop, stop} ->
            stop(Kernel)
    end.

%% Stop the kernel process as the stop of the kernel application stops
%% kernel_sup, then halt() (init.erl:943-945). init shuts down
%% application_controller, which stops the running applications
%% (init.erl:955-962). The PBeam boot does not start the kernel application
%% (see "Boot" in docs/erts-differences.md), so the kernel process stands in
%% for it. A boot that has not started its kernel process yet halts at once.
stop(undefined) ->
    halt();
stop(Kernel) ->
    Ref = monitor(process, Kernel),
    Kernel ! {self(), stop},
    receive {'DOWN', Ref, process, Kernel, _} -> halt() end.

%% init.erl:1808-1819. The flag values are already strings.
get_argument(Arg, Flags) ->
    case get_argument1(Arg, Flags) of
        [] -> error;
        Value -> {ok, Value}
    end.

get_argument1(Arg, [{Arg, V} | Args]) -> [V | get_argument1(Arg, Args)];
get_argument1(Arg, [_ | Args]) -> get_argument1(Arg, Args);
get_argument1(_, []) -> [].

%% Do the kernel part of kernel:start/2. Run logger:internal_init_logger/0
%% (kernel.erl:37). Do not run os:internal_init_cmd_shell/0 (kernel.erl:38):
%% only os:cmd/1,2 reads the shell that it puts in a persistent term
%% (os.erl:602, 630-669). Start the kernel_sup children StdError, File,
%% SigSrv, User, and LoggerSup in the order of kernel.erl:117-168, 188-194,
%% and 222-225 for a node without -master. File is file_server_2, which
%% file:read_file_info/1 and os:find_executable/1 call. Then run
%% erl_signal_handler:start/0 and logger:add_handlers(kernel)
%% (kernel.erl:41-42). Trap exits, as a supervisor does
%% (supervisor.erl:950), and stay alive as the link parent of the servers.
%% At a stop request of init, shut down the children in the reverse order
%% with their kernel_sup shutdown times (kernel.erl:124-168), as a
%% supervisor stops its children (supervisor.erl:1574-1597), and exit.
kernel(Boot) ->
    process_flag(trap_exit, true),
    ok = logger:internal_init_logger(),
    {ok, StdError} = standard_error:start_link(),
    {ok, File} = file_server:start_link(),
    {ok, SigSrv} = gen_event:start_link({local, erl_signal_server}),
    {ok, User} = user_sup:start(),
    {ok, LoggerSup} = logger_sup:start_link(),
    ok = erl_signal_handler:start(),
    ok = logger:add_handlers(kernel),
    Boot ! {self(), started},
    receive
        {_Init, stop} ->
            [shutdown(Pid, Time)
             || {Pid, Time} <- [{LoggerSup, infinity}, {User, 2000}, {SigSrv, 2000},
                                {File, 2000}, {StdError, 2000}]]
    end.

%% Shut down a child as supervisor:shutdown/1 does (supervisor.erl:1627-1656):
%% send the exit signal shutdown, kill the child when it does not exit in
%% Time ms, and then remove the link. It writes no shutdown_error report.
shutdown(Pid, Time) ->
    Ref = monitor(process, Pid),
    exit(Pid, shutdown),
    receive
        {'DOWN', Ref, process, Pid, _} -> ok
    after Time ->
        exit(Pid, kill),
        receive {'DOWN', Ref, process, Pid, _} -> ok end
    end,
    unlink(Pid).

%% The kernel application spec of the pinned start_clean.boot.
kernel_app() ->
    {application,kernel,
                 [{description,"ERTS  CXC 138 10"},
                  {vsn,"11.0.4"},
                  {id,[]},
                  {modules,[application,application_controller,application_master,
                            application_starter,auth,code,code_server,
                            data_publisher,dist_util,erl_boot_server,
                            erl_compile_server,erl_debugger,erl_distribution,
                            erl_erts_errors,erl_reply,erl_kernel_errors,
                            erl_signal_handler,erpc,error_handler,error_logger,
                            file,file_server,file_io_server,global,global_group,
                            global_search,group,group_history,heart,inet6_tcp,
                            inet6_tcp_dist,inet6_udp,inet6_sctp,inet_config,
                            inet_epmd_dist,inet_epmd_socket,inet_hosts,
                            inet_gethost_native,inet_tcp_dist,kernel,
                            kernel_config,kernel_refc,local_tcp,local_udp,logger,
                            logger_backend,logger_config,logger_disk_log_h,
                            logger_filters,logger_formatter,logger_h_common,
                            logger_handler,logger_handler_watcher,logger_olp,
                            logger_proxy,logger_server,logger_simple_h,
                            logger_std_h,logger_sup,net,net_adm,net_kernel,os,
                            ram_file,rpc,user_drv,user_sup,prim_tty,
                            prim_tty_sighandler,disk_log,disk_log_1,
                            disk_log_server,disk_log_sup,dist_ac,erl_ddll,
                            erl_epmd,erts_debug,gen_tcp,gen_tcp_socket,gen_udp,
                            gen_udp_socket,gen_sctp,inet,inet_db,inet_dns,
                            inet_dns_tsig,inet_parse,inet_res,inet_tcp,inet_udp,
                            inet_sctp,pg,pg2,raw_file_io,raw_file_io_compressed,
                            raw_file_io_deflate,raw_file_io_delayed,
                            raw_file_io_inflate,raw_file_io_list,seq_trace,socket,
                            standard_error,trace,wrap_log_reader]},
                  {registered,[application_controller,erl_reply,auth,boot_server,
                               code_server,disk_log_server,disk_log_sup,
                               erl_prim_loader,error_logger,file_server_2,
                               fixtable_server,global_group,global_name_server,
                               heart,init,kernel_config,kernel_refc,kernel_sup,
                               logger,logger_handler_watcher,logger_sup,
                               net_kernel,net_sup,rex,user,os_server,ddll_server,
                               erl_epmd,inet_db,pg]},
                  {applications,[]},
                  {optional_applications,[]},
                  {included_applications,[]},
                  {env,[{logger_level,notice},
                        {logger_sasl_compatible,false},
                        {net_tickintensity,4},
                        {net_ticktime,60},
                        {prevent_overlapping_partitions,true},
                        {shell_docs_ansi,auto},
                        {shell_history_drop,[]}]},
                  {maxT,infinity},
                  {maxP,infinity},
                  {mod,{kernel,[]}}]}.
