%% Operations that the policy of examples/restricted_guest does not permit.
%% The pbeam binary runs pbeam_boot:start(restricted_guest_probe, denied)
%% with the policy 'Elixir.RestrictedGuest.Policy', so this function runs
%% in the root process with the capabilities of initial/1 of the policy,
%% as RestrictedGuest.main/0 does. Do not run it on ERTS: there each
%% operation has its effect, and halt/1 stops the runtime.
-module(restricted_guest_probe).
-export([denied/0]).

%% Exit with {result, Results}: the result of each operation, or denied
%% for error:capability_denied. The first operation is permitted: a spawn
%% of a module of the guest, whose worker writes "worker 0: hello" to the
%% terminal. The pbeam binary prints the exit reason.
%% The build of the example has no module that formats a term, such as
%% io_lib_pretty, so the probe does not print the results itself.
denied() ->
    User = whereis(user),
    Results = [
        {spawn_own_module, own_worker()},
        {spawn_other_module, result(fun() -> spawn(lists, seq, [1, 2]) end)},
        {getenv, result(fun() -> os:getenv("HOME") end)},
        {open_port, result(fun() -> open_port({spawn_executable, "/bin/sh"}, [binary]) end)},
        {halt, result(fun() -> halt(0) end)},
        {handle_other_signal, result(fun() -> os:set_signal(sigusr2, handle) end)},
        {ignore_signal, result(fun() -> os:set_signal(sigwinch, ignore) end)},
        {exit_other, result(fun() -> exit(User, kill) end)},
        {group_leader_other, result(fun() -> group_leader(self(), User) end)},
        {request, result(fun() -> pbeam_cap:request(env, home) end)}
    ],
    exit({result, Results}).

own_worker() ->
    Worker = spawn('Elixir.RestrictedGuest.Worker', run, [self(), 0]),
    receive
        {done, Worker} -> done
    end.

result(Fun) ->
    try
        Fun()
    catch
        error:capability_denied -> denied
    end.
