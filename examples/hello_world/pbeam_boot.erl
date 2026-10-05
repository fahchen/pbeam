-module(pbeam_boot).
-export([start/2, init/2, kernel/1]).

%% Run the boot and then Module:Function(), as the init boot process runs
%% start_clean.boot and then -eval (init.erl:1192-1217). The boot process
%% traps exits (init.erl:1197). It registers the init request process, and
%% starts the kernel processes logger and application_controller in the
%% order of start_clean.boot (init.erl:1348-1350). It then runs the kernel
%% part of kernel:start/2, and sets the registered user process as the
%% group leader before it starts -eval (init.erl:1539-1546).
start(Module, Function) ->
    process_flag(trap_exit, true),
    Init = spawn(?MODULE, init, [self(), [{noshell, []}]]),
    true = register(init, Init),
    ok = start_in_kernel(Init, logger, logger_server, start_link, []),
    ok = start_in_kernel(Init, application_controller,
                         application_controller, start, [kernel_app()]),
    Kernel = spawn(?MODULE, kernel, [self()]),
    receive {Kernel, started} -> ok end,
    group_leader(whereis(user), self()),
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
%% get_arguments and {get_argument, Arg} (init.erl:816-819). It keeps other
%% messages in its mailbox.
init(Boot, Flags) ->
    process_flag(trap_exit, true),
    init_loop(Boot, Flags).

init_loop(Boot, Flags) ->
    receive
        {Boot, started, {_Server, {ok, Pid}}} when is_pid(Pid) ->
            link(Pid),
            Boot ! {self(), ok, Pid};
        {From, get_arguments} ->
            From ! {init, Flags};
        {From, {get_argument, Arg}} ->
            From ! {init, get_argument(Arg, Flags)}
    end,
    init_loop(Boot, Flags).

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
%% it needs file:read_file_info/2, and only os:cmd/1,2 reads the shell that
%% it puts in a persistent term (os.erl:602, 630-669). Start the kernel_sup
%% children StdError, SigSrv, User, and LoggerSup in the order of
%% kernel.erl:124-168 and 224-225. Then run
%% erl_signal_handler:start/0 and logger:add_handlers(kernel)
%% (kernel.erl:41-42). Trap exits, as a supervisor does
%% (supervisor.erl:950), and stay alive as the link parent of the servers.
kernel(Boot) ->
    process_flag(trap_exit, true),
    ok = logger:internal_init_logger(),
    {ok, _} = standard_error:start_link(),
    {ok, _} = gen_event:start_link({local, erl_signal_server}),
    {ok, _} = user_sup:start(),
    {ok, _} = logger_sup:start_link(),
    ok = erl_signal_handler:start(),
    ok = logger:add_handlers(kernel),
    Boot ! {self(), started},
    receive after infinity -> ok end.

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
