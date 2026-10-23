-module(pbeam_boot).
-export([start/2, start/3, init/2]).

%% Run the boot with no boot flags.
start(Module, Function) ->
    start(Module, Function, []).

%% Run the boot and then Module:Function(), as the init boot process runs
%% start_clean.boot and then -eval (init.erl:1192-1217). Flags are the boot
%% flags of the command line, as init:parse_boot_args/1 gives them
%% (init.erl:1668-1709): a list of {Flag, Values}, with each value a
%% binary. The boot first starts the pinned erts_code_purger, which
%% registers itself, as ERTS starts it as a system process next to init
%% (erl_init.c:2550-2557, erts_code_purger.erl:36-39). code_server asks it
%% to finish the load of a module with an -on_load function
%% (code_server.erl:1360-1362). The boot process traps exits
%% (init.erl:1197). It registers the
%% init request process with the flags after {noshell, []}, as for
%% erl -noshell. It then needs the root flag, as init does: without it, the
%% boot process exits with no_or_multiple_root_variables (init.erl:1198,
%% 1226-1232). It starts erl_prim_loader with the code path of the pa and
%% pz flags (init.erl:1200-1202) and sets the code path of
%% start_clean.boot (init.erl:1323-1332). It then runs the other commands
%% of start_clean.boot in their order (init.erl:1335-1361): the
%% kernelProcess commands of logger and application_controller, the load
%% of stdlib, application:start_boot/2 of kernel and stdlib, and
%% c:erlangrc/0. kernel:start/2 then starts kernel_sup and its children
%% (kernel.erl:35-45, 91-227). The boot starts no kernel process by hand.
%% It then sets the registered user process as the group leader before it
%% starts -eval (init.erl:1539-1546). In a pbeam run in boot mode (--boot),
%% the process then registers the IO servers user and standard_error as IO
%% services, so that a runtime process can send them the IO requests that
%% its terminal capability permits, and enters the runtime: it drops the
%% boot authority and gets the capabilities that the policy gives
%% Module:Function/0. In each other run, pbeam_cap:serve/2 and
%% pbeam_cap:enter_runtime/2 return ok and have no effect. ERTS has no
%% pbeam_cap module.
start(Module, Function, Flags) ->
    _ = spawn(erts_code_purger, start, []),
    process_flag(trap_exit, true),
    Init = spawn(?MODULE, init, [self(), [{noshell, []} | Flags]]),
    true = register(init, Init),
    Root = get_root(Flags),
    ok = start_prim_loader(Init, path_flags(Flags)),
    ok = boot_path(Root, path_flags(Flags)),
    ok = start_in_kernel(Init, logger, logger_server, start_link, []),
    ok = start_in_kernel(Init, application_controller,
                         application_controller, start, [kernel_app()]),
    ok = application:load(stdlib_app()),
    ok = application:start_boot(kernel, permanent),
    ok = application:start_boot(stdlib, permanent),
    _ = c:erlangrc(),
    group_leader(whereis(user), self()),
    ok = pbeam_cap:serve(whereis(user), io),
    ok = pbeam_cap:serve(whereis(standard_error), io),
    ok = pbeam_cap:enter_runtime(Module, Function),
    Result = Module:Function(),
    at_exit(),
    Result.

%% When the elixir application runs, finish as elixir -e does after its
%% commands: 'Elixir.Kernel.CLI':run/1 runs the at_exit callbacks, calls
%% Logger.flush() when the module Logger is loaded, and then calls
%% System.halt(Status) (kernel/cli.ex:58-71, 129-133). Logger.flush()
%% calls filesync/1 of each handler that exports it, so the queue of the
%% async default handler is written (logger.ex:785-796). The table
%% elixir_config holds the callbacks; without the elixir application,
%% the root process returns as after erl -eval.
at_exit() ->
    case ets:whereis(elixir_config) of
        undefined -> ok;
        _ -> 'Elixir.Kernel.CLI':run(fun(_Status) -> ok end)
    end.

%% init.erl:1226-1232.
get_root(Flags) ->
    case get_argument(root, Flags) of
        {ok, [[Root]]} -> Root;
        _ -> exit(no_or_multiple_root_variables)
    end.

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
%% so the directories stay as they are (init.erl:645-655, 1450-1451).
boot_path(Root, {Pa, Pz}) ->
    Lib = Root ++ "/lib/",
    erl_prim_loader:set_path(Pa ++ [Lib ++ "kernel-11.0.4/ebin", Lib ++ "stdlib-8.1/ebin"] ++ Pz).

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
%% kernel process that starts with {ok, Pid} and keeps it in the kernel list,
%% the last started first (init.erl:768-772), and answers the requests of
%% request/4. For init:stop/0 it runs stop/4 (init.erl:536, 788-789). When
%% a kernel process exits, it runs terminate/3 (init.erl:783-787). It
%% keeps other messages in its mailbox.
init(Boot, Flags) ->
    process_flag(trap_exit, true),
    init_loop(Boot, Flags, [], []).

init_loop(Boot, Flags, Kernel, Loaded) ->
    receive
        {Boot, started, {Server, {ok, Pid}}} when is_pid(Pid) ->
            link(Pid),
            Boot ! {self(), ok, Pid},
            init_loop(Boot, Flags, [{Server, Pid} | Kernel], Loaded);
        {stop, stop} ->
            stop(Boot, Kernel, Flags, Loaded);
        {'EXIT', Pid, Reason} ->
            terminate(Pid, Kernel, Reason),
            init_loop(Boot, Flags, Kernel, Loaded);
        {From, get_arguments = Request} ->
            init_loop(Boot, Flags, Kernel, request(From, Request, Flags, Loaded));
        {From, {get_argument, _} = Request} ->
            init_loop(Boot, Flags, Kernel, request(From, Request, Flags, Loaded));
        {From, fetch_loaded = Request} ->
            init_loop(Boot, Flags, Kernel, request(From, Request, Flags, Loaded));
        {From, {ensure_loaded, _} = Request} ->
            init_loop(Boot, Flags, Kernel, request(From, Request, Flags, Loaded))
    end.

%% Halt the node when a kernel process exits, as init:terminate/3 does
%% (init.erl:1139-1148): wait 500 ms for the error printouts, then call
%% halt/1 with the text "Kernel pid terminated (Name) (Reason)"
%% (init.erl:665-679). The exit of another linked process changes nothing.
terminate(Pid, Kernel, Reason) ->
    case lists:keyfind(Pid, 2, Kernel) of
        {Name, Pid} ->
            receive after 500 -> ok end,
            halt("Kernel pid terminated" ++ things_to_string([Name, Reason]));
        false ->
            false
    end.

%% init.erl:665-669.
things_to_string([X | Rest]) ->
    " (" ++ erts_internal:term_to_string(X, 32768) ++ ")" ++ things_to_string(Rest);
things_to_string([]) ->
    "".

%% Answer an init request and return the loaded list. get_arguments and
%% {get_argument, Arg} give the values as strings (init.erl:816-819).
%% fetch_loaded gives the modules that {ensure_loaded, Module} loaded
%% (init.erl:720-722, 738-750): the boot loads no module with primLoad,
%% because the pbeam binary loads the start modules. {ensure_loaded, Module}
%% reads the file of the module through erl_prim_loader and loads it with
%% erlang:load_module/2 (init.erl:730-750).
request(From, get_arguments, Flags, Loaded) ->
    From ! {init, get_arguments(Flags)},
    Loaded;
request(From, {get_argument, Arg}, Flags, Loaded) ->
    From ! {init, get_argument(Arg, Flags)},
    Loaded;
request(From, fetch_loaded, _Flags, Loaded) ->
    From ! {init, Loaded},
    Loaded;
request(From, {ensure_loaded, Module}, _Flags, Loaded) ->
    {Res, Loaded1} = ensure_loaded(Module, Loaded),
    From ! {init, Res},
    Loaded1.

%% Shut down the kernel processes and halt() as init:stop/2 does for
%% init:stop/0 (init.erl:924-945, 955-962, 981-987). init sends
%% {'EXIT', BootPid, shutdown} to each kernel process except logger, the
%% last started first, and waits for its exit (init.erl:1019-1035).
%% application_controller then stops the running applications, so
%% kernel_sup shuts down its children. While init waits, it ignores a
%% stop request, answers the init requests, and keeps the EXIT messages of
%% other processes (init.erl:1043-1062). The shutdown timer is infinity,
%% because PBeam has no shutdown_time flag (init.erl:1632-1646). The halt
%% then stops logger and each other process, in place of kill_all_pids/2
%% and exit(Logger, kill) (init.erl:960, 1072-1093).
stop(Boot, [{logger, _} | Kernel], Flags, Loaded) ->
    stop(Boot, Kernel, Flags, Loaded);
stop(Boot, [{_Server, Pid} | Kernel], Flags, Loaded) ->
    Pid ! {'EXIT', Boot, shutdown},
    stop(Boot, Kernel, Flags, shutdown_loop(Pid, Flags, Loaded));
stop(_Boot, [], _Flags, _Loaded) ->
    halt().

shutdown_loop(Pid, Flags, Loaded) ->
    receive
        {'EXIT', Pid, _} ->
            Loaded;
        {stop, _} ->
            shutdown_loop(Pid, Flags, Loaded);
        {From, get_arguments = Request} ->
            shutdown_loop(Pid, Flags, request(From, Request, Flags, Loaded));
        {From, {get_argument, _} = Request} ->
            shutdown_loop(Pid, Flags, request(From, Request, Flags, Loaded));
        {From, fetch_loaded = Request} ->
            shutdown_loop(Pid, Flags, request(From, Request, Flags, Loaded));
        {From, {ensure_loaded, _} = Request} ->
            shutdown_loop(Pid, Flags, request(From, Request, Flags, Loaded))
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

%% The stdlib application spec of the pinned start_clean.boot.
stdlib_app() ->
    {application,stdlib,
                 [{description,"ERTS  CXC 138 10"},
                  {vsn,"8.1"},
                  {id,[]},
                  {modules,[argparse,array,base64,beam_lib,binary,c,calendar,
                            dets,dets_server,dets_sup,dets_utils,dets_v9,dict,
                            digraph,digraph_utils,edlin,edlin_key,
                            edlin_context,edlin_expand,edlin_type_suggestion,
                            epp,eval_bits,erl_abstract_code,erl_anno,erl_bits,
                            erl_compile,erl_error,erl_eval,erl_expand_records,
                            erl_features,erl_internal,erl_lint,erl_parse,
                            erl_posix_msg,erl_pp,erl_scan,erl_stdlib_errors,
                            erl_tar,error_logger_file_h,error_logger_tty_h,
                            escript,ets,file_sorter,filelib,filename,gb_trees,
                            gb_sets,gen,gen_event,gen_fsm,gen_server,
                            gen_statem,graph,io,io_ansi,io_lib,io_lib_format,
                            io_lib_fread,io_lib_pretty,json,lists,log_mf_h,
                            man_docs,maps,math,ms_transform,orddict,ordsets,
                            otp_internal,peer,pool,proc_lib,proplists,qlc,
                            qlc_pt,queue,rand,random,re,records,sets,shell,
                            shell_default,shell_docs,shell_docs_markdown,
                            slave,sofs,string,supervisor,supervisor_bridge,
                            sys,timer,unicode,unicode_util,uri_string,
                            win32reg,zip,zstd]},
                  {registered,[timer_server,rsh_starter,take_over_monitor,
                               pool_master,dets]},
                  {applications,[kernel]},
                  {optional_applications,[]},
                  {included_applications,[]},
                  {env,[]},
                  {maxT,infinity},
                  {maxP,infinity}]}.
