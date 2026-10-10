%% A policy module for policy_probe. The root process of each probe
%% function gets the capabilities of initial/1, and each request of
%% policy_probe names the answer of grant/3.
-module(erl_policy).
-export([initial/1, grant/3, noop/0]).

initial(#{initial_call := {policy_probe, bad_initial, 0}}) ->
    [#{kind => process}];
initial(#{initial_call := {policy_probe, atom_initial, 0}}) ->
    all;
initial(#{initial_call := {policy_probe, raise_initial, 0}}) ->
    error(policy_bug);
initial(#{initial_call := {policy_probe, loop_initial, 0}}) ->
    loop();
initial(#{initial_call := {policy_probe, exit_initial, 0}}) ->
    exit(self(), kill);
initial(#{initial_call := {policy_probe, wait_initial, 0}}) ->
    receive
        never -> []
    end;
initial(Requester) ->
    put(root, Requester),
    [
        #{
            kind => process,
            operations => [spawn, link, monitor, send],
            modules => [policy_probe],
            delegable => true
        }
    ].

grant(_Kind, {answer, Answer}, _Requester) ->
    Answer;
grant(env, home, _Requester) ->
    {grant, home()};
grant(_Kind, raise, _Requester) ->
    error(policy_bug);
grant(_Kind, exit, _Requester) ->
    exit(policy_exit);
grant(_Kind, loop, _Requester) ->
    loop();
grant(_Kind, self_request, _Requester) ->
    try pbeam_cap:request(env, home) of
        ok -> deny
    catch
        error:capability_denied -> {grant, home()}
    end;
grant(_Kind, requester, #{pid := Pid} = Requester) ->
    Pid ! {requester, Requester, get(root)},
    deny;
grant(process, effect, _Requester) ->
    true = is_pid(spawn(?MODULE, noop, [])),
    {grant, #{kind => process, operations => [spawn], modules => [erl_policy], delegable => false}};
grant(_Kind, {spin, Count}, _Requester) ->
    spin(Count),
    deny;
grant(_Kind, {spin_grant, Count}, _Requester) ->
    spin(Count),
    {grant, home()};
grant(_Kind, {wait_spin, Count}, _Requester) ->
    wait_spin(Count),
    {grant, home()};
grant(_Kind, wake, _Requester) ->
    Policy = self(),
    spawn(fun() -> Policy ! wake end),
    deny;
grant(_Kind, link_crash, _Requester) ->
    spawn_link(fun() -> exit(crash) end),
    deny;
grant(_Kind, {kill, Count}, _Requester) ->
    spin(Count),
    exit(self(), kill).

noop() -> ok.

home() ->
    #{kind => env, operations => [read], names => ["HOME"], delegable => false}.

loop() -> loop().

spin(0) -> ok;
spin(Count) -> spin(Count - 1).

%% Each step uses 3000 reductions and then waits 1 ms in a receive, so the
%% policy process switches out in the receive and not at the end of its
%% budget.
wait_spin(0) ->
    ok;
wait_spin(Count) ->
    spin(3000),
    receive
    after 1 -> ok
    end,
    wait_spin(Count - 1).
