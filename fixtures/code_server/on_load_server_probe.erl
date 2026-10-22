-module(on_load_server_probe).
-export([main/0, failure/0, hibernated/0, killed/0, survived/0, wait/0,
         system_tasks/0, fake_purger/0, purge_loaded/0, purge_prepare/0,
         gc_task/0]).

%% Write the results of code_server for modules with an -on_load function
%% whose function returns ok, one term on each line. code:ensure_loaded/1
%% loads on_load_ok, and a call loads on_load_call on demand through
%% error_handler (error_handler.erl:85-98; code.erl:581-600). code_server
%% runs the on_load function in a new process and makes the code current
%% through erts_code_purger (code_server.erl:1348-1365, 1386-1403;
%% erts_code_purger.erl:166-170).
main() ->
    Ensured = [erlang:module_loaded(on_load_ok), code:ensure_loaded(on_load_ok),
               erlang:module_loaded(on_load_ok), on_load_ok:value(),
               code:ensure_loaded(on_load_ok)],
    Called = [erlang:module_loaded(on_load_call), on_load_call:value(),
              erlang:module_loaded(on_load_call)],
    io:format("~p~n", [[Ensured, Called]]).

%% Write the results of code_server for modules with an -on_load function
%% that fails: on_load_error returns an error, and on_load_crash raises.
%% code_server drops the code through erts_code_purger, and the load gives
%% {error, on_load_failure} (code_server.erl:1360-1368).
failure() ->
    Failed = [code:ensure_loaded(on_load_error), erlang:module_loaded(on_load_error),
              try on_load_crash:value() catch Class:Reason -> {Class, Reason} end,
              erlang:module_loaded(on_load_crash)],
    io:format("~p~n", [Failed]).

%% Write the result of an on_load load through an erts_code_purger that
%% hibernated as its idle timeout does: after 60 seconds with no request,
%% wait_for_request/0 calls erlang:hibernate(erts_code_purger,
%% wait_for_request, []) (erts_code_purger.erl:40-45). The probe starts a
%% process that makes the same call, lets it run to the call with one
%% scheduler, and registers it as erts_code_purger. code_server then makes
%% the code of on_load_ok current through it (erts_code_purger.erl:55-59,
%% 166-170).
hibernated() ->
    Probe = self(),
    Purger = spawn(fun() ->
                           Probe ! hibernating,
                           erlang:hibernate(erts_code_purger, wait_for_request, [])
                   end),
    receive hibernating -> ok end,
    erlang:yield(),
    true = unregister(erts_code_purger),
    true = register(erts_code_purger, Purger),
    Loaded = [code:ensure_loaded(on_load_ok), on_load_ok:value(),
              erlang:is_process_alive(Purger)],
    io:format("~p~n", [Loaded]).

%% Write the result of a load of on_load_spawn, whose on_load function
%% starts a process in the code of the module and fails. erts_code_purger
%% kills the process before it purges the code, and the load gives
%% {error, on_load_failure} (erts_code_purger.erl:166-182, 249-265).
killed() ->
    true = register(on_load_spawn_probe, self()),
    Result = code:ensure_loaded(on_load_spawn),
    Pid = receive {waiting, P} -> P end,
    io:format("~p~n", [[Result, erlang:is_process_alive(Pid),
                        erlang:module_loaded(on_load_spawn)]]).

%% Write the result of a load of on_load_survive, whose on_load function
%% starts a process that leaves the code of the module. The process has no
%% frame in the code, so the check_process_code request answers false for
%% it and erts_code_purger does not kill it (erts_code_purger.erl:166-182,
%% 249-265). The process runs on, also with an erlang:apply/3 call, after
%% the purge removed the code (beam_bif_load.c:2249-2262).
survived() ->
    true = register(on_load_spawn_probe, self()),
    Result = code:ensure_loaded(on_load_survive),
    Pid = receive {waiting, P} -> P end,
    Alive = erlang:is_process_alive(Pid),
    Pid ! {apply, self(), lists, reverse, [[1, 2, 3]]},
    Applied = receive {applied, Pid, Value} -> Value end,
    io:format("~p~n", [[Result, Alive, Applied]]).

%% Wait for a request of a process that the on_load function of
%% on_load_survive started. The call of erlang:apply/3 reads the start MFA
%% of the process, whose code on_load_survive no longer has.
wait() ->
    receive
        {apply, From, Module, Function, Arguments} ->
            From ! {applied, self(), erlang:apply(Module, Function, Arguments)},
            wait();
        stop ->
            ok
    end.

%% Write the results of the BIFs that erts_code_purger calls, from a
%% process that is not erts_code_purger: erts_internal:purge_module/2
%% raises notsup (beam_bif_load.c:2090-2092); the argument checks of
%% erts_internal:request_system_task/3 (erl_process.c:11309-11492) and
%% erts_internal:processes_next/1 (bif.c:4107-4118, erl_ptab.c:1472-1534);
%% the reply of a check_process_code request; and
%% erlang:system_info(outstanding_system_requests_limit) (erl_init.c:1354).
system_tasks() ->
    Self = self(),
    Dead = spawn(fun() -> ok end),
    Ref = monitor(process, Dead),
    receive {'DOWN', Ref, process, Dead, _} -> ok end,
    Request = fun(Pid, Prio, Req) ->
                      try erts_internal:request_system_task(Pid, Prio, Req)
                      catch Class:Reason -> {Class, Reason} end
              end,
    Next = fun(Arg) ->
                   try erts_internal:processes_next(Arg)
                   catch Class:Reason -> {Class, Reason} end
           end,
    Badargs = [Request(self, normal, {check_process_code, a, lists}),
               Request(Self, urgent, {check_process_code, a, lists}),
               Request(Self, 1, {check_process_code, a, lists}),
               Request(Self, normal, check_process_code),
               Request(Self, normal, {check_process_code}),
               Request(Self, normal, {check_process_code, a, lists, b, c}),
               Request(Self, normal, {"check_process_code", a, lists}),
               Request(Self, normal, {unknown_task, a, lists}),
               Request(Self, normal, {check_process_code, a}),
               Request(Self, normal, {check_process_code, a, "lists"}),
               Request(Self, normal, {garbage_collect, a}),
               Request(Self, normal, {garbage_collect, a, invalid}),
               Next(-1), Next(1048577), Next(1 bsl 64), Next(none)],
    Replies = [Request(Self, inherit, {check_process_code, self_tag, lists}),
               Request(Dead, max, {check_process_code, {dead_tag, 1}, lists, extra}),
               receive {check_process_code, self_tag, R1} -> R1 end,
               receive {check_process_code, {dead_tag, 1}, R2} -> R2 end],
    %% More processes than reductions in one turn, so that one call of
    %% processes_next/1 gives at most one pid for each reduction left.
    Waiting = [spawn(fun() -> receive stop -> ok end end) || _ <- lists:seq(1, 5000)],
    All = all_pids(erlang:processes_iterator(), []),
    Listed = processes(),
    [Pid ! stop || Pid <- Waiting],
    Iterated = [Next(1048576), lists:sort(All) =:= lists:sort(Listed),
                erlang:system_info(outstanding_system_requests_limit)],
    Notsup = try erts_internal:purge_module(lists, complete)
             catch C:R -> {C, R} end,
    io:format("~p~n", [[Badargs, Replies, Iterated, Notsup]]).

all_pids(Iter, Acc) ->
    case erlang:processes_next(Iter) of
        none -> Acc;
        {Pid, Next} -> all_pids(Next, [Pid | Acc])
    end.

%% PBeam only: take the name erts_code_purger, which PBeam checks for the
%% caller of erts_internal:purge_module/2, and write the results of
%% purge_module/2 for each argument check (beam_bif_load.c:2088-2337). On
%% ERTS, only the system process erts_code_purger can call the BIF.
fake_purger() ->
    Purger = whereis(erts_code_purger),
    true = unregister(erts_code_purger),
    true = register(erts_code_purger, self()),
    Purge = fun(Mod, Op) ->
                    try erts_internal:purge_module(Mod, Op)
                    catch Class:Reason -> {Class, Reason} end
            end,
    {ok, Bin} = file:read_file(code:which(on_load_error)),
    Prepared = erlang:prepare_loading(on_load_error, Bin),
    OnLoad = erlang:finish_loading([Prepared]),
    Kept = erlang:finish_after_on_load(on_load_error, false),
    Results = [OnLoad, Kept,
               Purge(1, complete), Purge(on_load_error, unknown),
               Purge(on_load_error, complete),
               Purge(on_load_server_probe_missing, prepare_on_load),
               Purge(on_load_error, prepare_on_load),
               Purge(on_load_error, prepare_on_load),
               Purge(lists, complete),
               Purge(on_load_error, complete),
               Purge(on_load_error, complete),
               erlang:module_loaded(on_load_error)],
    true = unregister(erts_code_purger),
    true = register(erts_code_purger, Purger),
    io:format("~p~n", [[Results, code:ensure_loaded(on_load_error)]]).

%% PBeam only: erts_internal:purge_module(lists, prepare_on_load) of a
%% loaded module with no on_load code stops the run, where the ERTS
%% assertion fails (beam_bif_load.c:2134-2140).
purge_loaded() ->
    fake_purger_call(lists, prepare_on_load).

%% PBeam only: the purge operation prepare gives false for a module with
%% no old code (beam_bif_load.c:2143-2144). PBeam does not purge the old
%% code that erlang:delete_module/1 leaves, so prepare of such a module
%% stops the run.
purge_prepare() ->
    false = fake_purger_call(lists, prepare),
    true = erlang:delete_module(on_load_server_probe),
    erts_internal:purge_module(on_load_server_probe, prepare).

fake_purger_call(Mod, Op) ->
    true = unregister(erts_code_purger),
    true = register(erts_code_purger, self()),
    erts_internal:purge_module(Mod, Op).

%% PBeam only: the garbage_collect system task stops the run.
gc_task() ->
    erts_internal:request_system_task(self(), normal, {garbage_collect, a, major}).
