%% A probe of boot mode (pbeam --boot --policy boot_policy). The root
%% process starts with the boot authority, enters the runtime with
%% pbeam_cap:enter_runtime/2, and then gets the capabilities that
%% initial/1 of boot_policy gives. ERTS has no pbeam_cap module and no
%% capability, so the probe runs only on pbeam.
-module(boot_authority_probe).
-export([main/0, denied/0, child_enter/0, serve/0, service/0, worker/0]).

-define(UNSET, "PBEAM_BOOT_AUTHORITY_PROBE_UNSET").

%% The root process does protected effects with no capability copies,
%% starts the boot process Service, and then enters the runtime. As a
%% runtime process, it gets error:capability_denied for each operation on
%% Service except a monitor and a message to an alias of Service. The same
%% operations on the runtime process Worker are permitted. The root process
%% exits with {result, Results}.
main() ->
    Root = self(),
    Service = spawn(?MODULE, service, []),
    true = register(boot_authority_service, Service),
    Service ! {alias, Root},
    Alias =
        receive
            {alias, A} -> A
        end,
    Boot = [
        {boot_getenv, result(fun() -> os:getenv(?UNSET) end)},
        {boot_spawn_restricted,
            result(fun() -> spawn_opt(?MODULE, worker, [], [{capabilities, #{}}]) end)},
        {boot_request, result(fun() -> pbeam_cap:request(env, home) end)},
        {boot_enter_badarg, result(fun() -> pbeam_cap:enter_runtime(1, guest) end)}
    ],
    ok = pbeam_cap:enter_runtime(?MODULE, guest),
    Worker = spawn(?MODULE, worker, []),
    Runtime = [
        {enter_again, pbeam_cap:enter_runtime(?MODULE, again)},
        {requester, requester()},
        {getenv, result(fun() -> os:getenv(?UNSET) end)},
        {spawn_other, result(fun() -> spawn(lists, reverse, [[]]) end)},
        {send_pid, result(fun() -> Service ! x end)},
        {send_name, result(fun() -> boot_authority_service ! x end)},
        {timer_pid, result(fun() -> erlang:start_timer(0, Service, x) end)},
        {timer_name, result(fun() -> erlang:start_timer(0, boot_authority_service, x) end)},
        {link, result(fun() -> link(Service) end)},
        {exit, result(fun() -> exit(Service, kill) end)},
        {exit_alias, result(fun() -> exit(Alias, kill) end)},
        {group_leader, result(fun() -> group_leader(self(), Service) end)},
        {process_info, result(fun() -> process_info(Service, registered_name) end)},
        {is_process_alive, result(fun() -> is_process_alive(Service) end)},
        {unregister, result(fun() -> unregister(boot_authority_service) end)},
        {monitor, is_reference(monitor(process, Service))},
        {alias, alias_reply(Alias, via_alias)},
        {policy_revoke, policy_revoke(Service)},
        {boot_spawn_after_revoke, alias_reply(Alias, spawn)},
        {runtime_send, Worker ! x},
        {runtime_link, link(Worker)},
        {runtime_process_info, process_info(Worker, registered_name)},
        {runtime_unregister, register(boot_authority_guest, Root) andalso
            unregister(boot_authority_guest)}
    ],
    exit({result, Boot ++ Runtime}).

%% The boot root process registers the boot process Service as an IO
%% service with pbeam_cap:serve/2. A pid that is not a live boot process
%% and arguments that are not a pid and io raise badarg. After the root
%% process enters the runtime, the IO service entry of Service checks its
%% IO requests: boot_policy gives no terminal capability, so a put_chars
%% request is denied, and a requests request with no request passes. A
%% runtime process gets the argument checks of pbeam_cap:serve/2 too. The
%% root process exits with {result, Results}.
serve() ->
    Service = spawn(?MODULE, worker, []),
    Dead = spawn(?MODULE, worker, []),
    Ref = monitor(process, Dead),
    Dead ! stop,
    receive
        {'DOWN', Ref, process, Dead, normal} -> ok
    end,
    Boot = [
        {serve_not_pid, result(fun() -> pbeam_cap:serve(boot_authority_service, io) end)},
        {serve_not_io, result(fun() -> pbeam_cap:serve(Service, file) end)},
        {serve_dead, result(fun() -> pbeam_cap:serve(Dead, io) end)},
        {serve, pbeam_cap:serve(Service, io)},
        {serve_again, pbeam_cap:serve(Service, io)}
    ],
    ok = pbeam_cap:enter_runtime(?MODULE, guest),
    Runtime = [
        {put_chars, result(fun() -> Service ! {io_request, self(), x, {put_chars, unicode, <<"x">>}}, sent end)},
        {empty_requests, result(fun() -> Service ! {io_request, self(), x, {requests, []}}, sent end)},
        {runtime_serve_not_io, result(fun() -> pbeam_cap:serve(Service, file) end)}
    ],
    exit({result, Boot ++ Runtime}).

%% initial/1 of the policy denies the root process when it enters the
%% runtime, so the run stops with the error of a denied root process.
denied() ->
    pbeam_cap:enter_runtime(?MODULE, denied_guest),
    exit(entered).

%% A boot process that is not the root process cannot enter the runtime:
%% the run stops with a runtime error.
child_enter() ->
    spawn(fun() -> pbeam_cap:enter_runtime(?MODULE, guest) end),
    receive
    after infinity -> ok
    end.

%% A boot process. It gives an alias to the root process, answers the
%% messages that come through the alias, and spawns a process with no
%% capability copies.
service() ->
    receive
        {alias, From} ->
            Alias = monitor(process, From, [{alias, explicit_unalias}]),
            From ! {alias, Alias};
        {via_alias, From} ->
            From ! {service, via_alias};
        {spawn, From} ->
            From ! {service, is_pid(spawn(?MODULE, worker, []))}
    end,
    service().

worker() ->
    receive
        stop -> ok
    end.

%% Return the requester map that initial/1 got.
requester() ->
    {error, capability_denied} = result(fun() -> pbeam_cap:request(process, requester) end),
    receive
        {requester, Requester} -> Requester
    end.

%% Return the result of pbeam_cap:policy_revoke(Holder, process) in the
%% policy process.
policy_revoke(Holder) ->
    Request = {revoke, Holder, process},
    {error, capability_denied} = result(fun() -> pbeam_cap:request(process, Request) end),
    receive
        {revoked, Result} -> Result
    end.

%% Send {Request, self()} to Alias and return the answer of the service.
alias_reply(Alias, Request) ->
    Alias ! {Request, self()},
    receive
        {service, Reply} -> Reply
    end.

result(Fun) ->
    try
        case Fun() of
            Pid when is_pid(Pid) -> pid;
            Reference when is_reference(Reference) -> reference;
            Value -> Value
        end
    catch
        Class:Reason -> {Class, Reason}
    end.
