-module(pbeam_boot).
-export([start/2, start/3, init/2, kernel/1]).

%% Run the boot with no boot flags.
start(Module, Function) ->
    start(Module, Function, []).

%% Run the boot and then Module:Function(), as the init boot process runs
%% start_clean.boot and then -eval (init.erl:1192-1217). Flags are the boot
%% flags of the command line, as init:parse_boot_args/1 gives them
%% (init.erl:1668-1709): a list of {Flag, Values}, with each value a
%% binary. The boot process traps exits (init.erl:1197). It registers the
%% init request process with the flags after {noshell, []}, as for
%% erl -noshell. It starts erl_prim_loader with the code path of the pa and
%% pz flags (init.erl:1200-1202). With a root flag, it then sets the code
%% path of start_clean.boot (init.erl:1323-1332). It then starts the kernel
%% processes logger
%% and application_controller in the order of start_clean.boot
%% (init.erl:1348-1350). It then runs the kernel part of kernel:start/2,
%% and sets the registered user process as the group leader before it
%% starts -eval (init.erl:1539-1546). In a pbeam run in boot mode (--boot),
%% the process then registers the IO servers user and standard_error as IO
%% services, so that a runtime process can send them the IO requests that
%% its terminal capability permits, and enters the runtime: it drops the
%% boot authority and gets the capabilities that the policy gives
%% Module:Function/0. In each other run, pbeam_cap:serve/2 and
%% pbeam_cap:enter_runtime/2 return ok and have no effect. ERTS has no
%% pbeam_cap module.
start(Module, Function, Flags) ->
    process_flag(trap_exit, true),
    Init = spawn(?MODULE, init, [self(), [{noshell, []} | Flags]]),
    true = register(init, Init),
    ok = start_prim_loader(Init, path_flags(Flags)),
    ok = boot_path(get_flag_args(root, Flags), path_flags(Flags)),
    ok = start_in_kernel(Init, logger, logger_server, start_link, []),
    ok = start_in_kernel(Init, application_controller,
                         application_controller, start, [kernel_app()]),
    Kernel = spawn(?MODULE, kernel, [self()]),
    receive {Kernel, started} -> ok end,
    Init ! {self(), kernel, Kernel},
    group_leader(whereis(user), self()),
    ok = pbeam_cap:serve(whereis(user), io),
    ok = pbeam_cap:serve(whereis(standard_error), io),
    ok = pbeam_cap:enter_runtime(Module, Function),
    Module:Function().

%% Start the pinned erl_prim_loader with the path Pa ++ Pz ++ ["."], and
%% give its link to init (init.erl:1164-1184). PBeam has no -path flag, so
%% the path is always made from the pa and pz flags (init.erl:1165-1166,
%% 1200).
start_prim_loader(Init, {Pa, Pz}) ->
    Path = Pa ++ Pz ++ ["."],
    case erl_prim_loader:start() of
        {ok, Pid} ->
            erl_prim_loader:set_path(Path),
            add_to_kernel(Init, erl_prim_loader, {ok, Pid});
        {error, Reason} ->
            erlang:display({"cannot start loader", Reason}),
            exit(Reason)
    end.

%% Run the last path command of the pinned start_clean.boot,
%% {path, ["$ROOT/lib/kernel-11.0.4/ebin", "$ROOT/lib/stdlib-8.1/ebin"]}, as
%% init does when the boot has no -path flag: set the path to
%% Pa ++ BootPath ++ Pz, with $ROOT replaced by the root flag
%% (init.erl:1323-1332, 1417-1442). The default code_path_choice is strict,
%% so the directories stay as they are (init.erl:645-655, 1450-1451). A boot
%% without a root flag runs no boot script command: ERTS stops without it
%% (init.erl:1226-1232).
boot_path([[Root]], {Pa, Pz}) ->
    Lib = b2s(Root) ++ "/lib/",
    erl_prim_loader:set_path(Pa ++ [Lib ++ "kernel-11.0.4/ebin", Lib ++ "stdlib-8.1/ebin"] ++ Pz);
boot_path([], _) ->
    ok.

%% init.erl:1259-1262. The last pa flag comes first.
path_flags(Flags) ->
    Pa = append(reverse(get_flag_args(pa, Flags))),
    Pz = append(get_flag_args(pz, Flags)),
    {bs2ss(Pa), bs2ss(Pz)}.

%% Start a kernel process and give its link to init (init.erl:1519-1528).
start_in_kernel(Init, Server, Mod, Fun, Args) ->
    Res = apply(Mod, Fun, Args),
    add_to_kernel(Init, Server, Res).

%% Give the start result of a kernel process to init, which links to it, and
%% remove the link of the boot process (init.erl:1178-1184, 1521-1528).
add_to_kernel(Init, Server, Res) ->
    Init ! {self(), started, {Server, Res}},
    receive
        {Init, ok, Pid} ->
            unlink(Pid),
            ok
    end.

%% The init request process. It traps exits (init.erl:588), links to each
%% kernel process that starts with {ok, Pid} (init.erl:768-772), and answers
%% get_arguments and {get_argument, Arg} with the values as strings
%% (init.erl:816-819). It answers fetch_loaded with the modules that
%% {ensure_loaded, Module} loaded (init.erl:720-722, 738-750): the boot
%% loads no module with primLoad, because the pbeam binary loads the start
%% modules. It answers {ensure_loaded, Module} as init does
%% (init.erl:730-750): it reads the file of the module through
%% erl_prim_loader and loads it with erlang:load_module/2. For init:stop/0
%% it stops the kernel process and halts with status 0 (init.erl:536,
%% 788-789, 924-945). It keeps other messages in its mailbox.
init(Boot, Flags) ->
    process_flag(trap_exit, true),
    init_loop(Boot, Flags, undefined, []).

init_loop(Boot, Flags, Kernel, Loaded) ->
    receive
        {Boot, started, {_Server, {ok, Pid}}} when is_pid(Pid) ->
            link(Pid),
            Boot ! {self(), ok, Pid},
            init_loop(Boot, Flags, Kernel, Loaded);
        {Boot, kernel, Pid} ->
            init_loop(Boot, Flags, Pid, Loaded);
        {From, get_arguments} ->
            From ! {init, get_arguments(Flags)},
            init_loop(Boot, Flags, Kernel, Loaded);
        {From, {get_argument, Arg}} ->
            From ! {init, get_argument(Arg, Flags)},
            init_loop(Boot, Flags, Kernel, Loaded);
        {From, fetch_loaded} ->
            From ! {init, Loaded},
            init_loop(Boot, Flags, Kernel, Loaded);
        {From, {ensure_loaded, Module}} ->
            {Res, Loaded1} = ensure_loaded(Module, Loaded),
            From ! {init, Res},
            init_loop(Boot, Flags, Kernel, Loaded1);
        {stop, stop} ->
            stop(Kernel)
    end.

%% Load a module that the run does not hold, as init:ensure_loaded/2 does
%% (init.erl:730-737): a module that is loaded needs no file, and the
%% loaded list keeps the file of each module that this function loads
%% (init.erl:738-750). The file name is the module name with the object
%% file extension (init.erl:738-739, 1844-1847). The loader reads the file
%% with erl_prim_loader:get_file/1 and loads the module with
%% erlang:load_module/2, as init:do_load_module/2 does (init.erl:1614-1624).
%% A module that the loader does not load gives the load error of
%% erlang:load_module/2, and a file that the loader does not find gives the
%% atom error of erl_prim_loader:get_file/1 (init.erl:747).
ensure_loaded(Module, Loaded) ->
    case erlang:module_loaded(Module) of
        true ->
            {{module, Module}, Loaded};
        false ->
            File = atom_to_list(Module) ++ ".beam",
            case erl_prim_loader:get_file(File) of
                {ok, BinCode, FullName} ->
                    case erlang:load_module(Module, BinCode) of
                        {module, Module} -> {{module, Module}, [{Module, FullName} | Loaded]};
                        Error -> {Error, Loaded}
                    end;
                Error ->
                    {Error, Loaded}
            end
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

%% init.erl:1790-1819.
get_flag_args(F, Flags) -> get_flag_args(F, Flags, []).

get_flag_args(F, [{F, V} | Flags], Acc) ->
    get_flag_args(F, Flags, [V | Acc]);
get_flag_args(F, [_ | Flags], Acc) ->
    get_flag_args(F, Flags, Acc);
get_flag_args(_, [], Acc) ->
    reverse(Acc).

get_arguments([{F, V} | Flags]) ->
    [{F, to_strings(V)} | get_arguments(Flags)];
get_arguments([]) ->
    [].

to_strings([H | T]) when is_atom(H) -> [atom_to_list(H) | to_strings(T)];
to_strings([H | T]) when is_binary(H) -> [b2s(H) | to_strings(T)];
to_strings([]) -> [].

get_argument(Arg, Flags) ->
    case get_argument1(Arg, Flags) of
        [] -> error;
        Value -> {ok, Value}
    end.

get_argument1(Arg, [{Arg, V} | Args]) -> [to_strings(V) | get_argument1(Arg, Args)];
get_argument1(Arg, [_ | Args]) -> get_argument1(Arg, Args);
get_argument1(_, []) -> [].

%% init.erl:435-438, with a list comprehension in place of map/2.
bs2ss(L0) when is_list(L0) ->
    [b2s(B) || B <- L0];
bs2ss(L) ->
    L.

%% init.erl:630-637.
b2s(Bin) when is_binary(Bin) ->
    try
        unicode:characters_to_list(Bin, file:native_name_encoding())
    catch
        _:_ -> binary_to_list(Bin)
    end;
b2s(L) when is_list(L) ->
    L.

%% init.erl:1828-1840.
append([E]) -> E;
append([H | T]) ->
    H ++ append(T);
append([]) -> [].

reverse([] = L) ->
    L;
reverse([_] = L) ->
    L;
reverse([A, B]) ->
    [B, A];
reverse([A, B | L]) ->
    lists:reverse(L, [B, A]).

%% Do the kernel part of kernel:start/2. Run logger:internal_init_logger/0
%% (kernel.erl:37). Do not run os:internal_init_cmd_shell/0 (kernel.erl:38):
%% only os:cmd/1,2 reads the shell that it puts in a persistent term
%% (os.erl:602, 630-669). Start the kernel_sup children Code (only with a
%% root flag, see start_code/1), StdError, File, SigSrv, User, and LoggerSup
%% in the order of kernel.erl:110-168, 188-194, and 222-225 for a node
%% without -master. File is file_server_2, which
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
    Code = start_code(init:get_argument(root)),
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
                                {File, 2000}, {StdError, 2000} | Code]]
    end.

%% Start the code server, the first kernel_sup child (kernel.erl:110-115,
%% 222), when the boot has a root flag: code:start_link/0 reads it
%% (code.erl:1686). Return the child for shutdown/2, or no child.
start_code({ok, _}) ->
    {ok, Code} = code:start_link(),
    [{Code, 2000}];
start_code(error) ->
    [].

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
