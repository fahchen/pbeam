-module(kernel_boot_probe).
-behaviour(application).
-behaviour(supervisor).
-export([main/0, main/1, lookup/0, gethost/0, supports/0, start/2, stop/1, init/1]).

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

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% lookup -kernel logger_level warning` without a domain in an inetrc file
%% runs lookup/0. On Linux, inet_config:init/0 looked up the host name at
%% the kernel start (inet_config.erl:110-133). lookup/0 displays the domain
%% after that, and the results of the native lookup of the host name and of
%% localhost through the inet_gethost port program
%% (inet_gethost_native.erl:412-428), and then calls init:stop/0.
lookup() ->
    Name = inet_db:gethostname(),
    erlang:display({lookup, inet_db:res_option(lookup), inet_db:res_option(domain),
                    inet:gethostbyname(Name), inet:gethostbyname("localhost"),
                    is_pid(whereis(inet_gethost_native))}),
    init:stop().

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% gethost -kernel logger_level warning` runs gethost/0. It displays the
%% replies of the inet_gethost port program (inet_gethost.c:180-249) to
%% requests that it answers, discards, or ends on, and the results of
%% inet_gethost_native and inet for localhost, addresses, and an unknown
%% name, and then calls init:stop/0.
gethost() ->
    Ask = fun(Requests, Timeout) ->
                  Port = gethost_port(),
                  [begin true = port_command(Port, R),
                         receive {Port, M} -> M after Timeout -> none end
                   end || R <- Requests]
          end,
    Discarded = Ask([<<1:32, 3>>, <<16#FFFFFFFF:32, 4, 0, 0:32>>, <<2:32, 9, 1>>,
                     <<3:32, 1, 7, "x", 0>>, <<4:32, 1, 1, "localhost">>,
                     <<5:32, 2, 1, 1, 2, 3>>], 300),
    Answered = Ask([<<6:32, 1, 1, "localhost", 0>>, <<7:32, 2, 2, 0:120, 1>>,
                    <<8:32, 2, 1, 127, 0, 0, 1>>,
                    <<9:32, 1, 2, "no-such-host.invalid", 0>>,
                    <<10:32, 2, 2, 16#20010db8:32, 0:96>>], 20000),
    Drain = fun Drain(Port, Timeout) ->
                    receive {Port, M} -> [M | Drain(Port, 1000)]
                    after Timeout -> []
                    end
            end,
    Fatal = [begin Port = gethost_port(), true = port_command(Port, R), Drain(Port, 20000) end
             || R <- [<<1, 2, 3>>, <<1:32, 4, 0, 1:32>>]],
    Native = [inet_gethost_native:gethostbyname("localhost"),
              inet_gethost_native:gethostbyname("localhost", inet6),
              inet_gethost_native:gethostbyname("127.0.0.1"),
              inet_gethost_native:gethostbyname("no-such-host.invalid"),
              inet_gethost_native:gethostbyaddr("127.0.0.1"),
              inet_gethost_native:gethostbyaddr("::1"),
              inet:gethostbyname("localhost")],
    erlang:display({gethost, Discarded, Answered, Fatal, Native}),
    init:stop().

gethost_port() ->
    {ok, [BinDir]} = init:get_argument(bindir),
    open_port({spawn_executable, filename:join(BinDir, "inet_gethost")},
              [{args, ["4"]}, {packet, 4}, eof, exit_status, binary]).

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% supports -kernel logger_level warning` runs supports/0. erl_init:start/2
%% ran prim_socket:on_load/0, which loads the NIF library and stores the
%% tables of nif_supports/1 as persistent terms (erl_init.erl:42-48,
%% prim_socket.erl:137-204). supports/0 displays the result of
%% nif_supports/0, each table as a sorted list, and the results of other
%% keys, and then calls init:stop/0.
supports() ->
    Tables = [{Key, lists:sort(maps:to_list(persistent_term:get({prim_socket, Key})))}
              || Key <- [protocols, options, ioctl_requests, ioctl_flags, msg_flags]],
    erlang:display({supports, prim_socket:supports(), Tables,
                    prim_socket:supports(other), prim_socket:supports("options"),
                    prim_socket:is_supported(local), prim_socket:is_supported(other),
                    prim_socket:is_supported(options, {socket, reuseaddr}),
                    prim_socket:is_supported(other, local)}),
    init:stop().

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
