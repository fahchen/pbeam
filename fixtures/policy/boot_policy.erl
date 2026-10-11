%% A policy module for boot_authority_probe in boot mode. initial/1 runs
%% when the root process enters the runtime. It gives each process
%% operation for the probe module, so the runtime to boot gate, not a
%% missing capability, denies each operation on a boot process. grant/3
%% denies each request.
-module(boot_policy).
-export([initial/1, grant/3]).

initial(#{initial_call := {boot_authority_probe, denied_guest, 0}}) ->
    deny;
initial(Requester) ->
    put(root, Requester),
    [
        #{
            kind => process,
            operations => [spawn, link, monitor, exit_other, send, inspect, group_leader],
            modules => [boot_authority_probe],
            delegable => true
        }
    ].

grant(_Kind, requester, #{pid := Pid}) ->
    Pid ! {requester, get(root)},
    deny;
grant(_Kind, {revoke, Holder, Kind}, #{pid := Pid}) ->
    Pid ! {revoked, pbeam_cap:policy_revoke(Holder, Kind)},
    deny;
grant(_Kind, _Request, _Requester) ->
    deny.
