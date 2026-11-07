-module(dist_global_pg_probe).
-export([server/0, run/2]).

server() ->
    register(cluster_probe, self()),
    {ok, _} = pg:start_link(),
    {ok, _} = pg:start_link(cluster_scope),
    yes = global:register_name({initial, node()}, self()),
    io:format("~s~n", [node()]),
    receive {ping, From} -> From ! pong, receive stop -> ok end; stop -> ok end.

run(A, B) ->
    true = net_kernel:connect_node(A),
    true = net_kernel:connect_node(B),
    {ok, _} = pg:start_link(),
    {ok, _} = pg:start_link(cluster_scope),
    ok = global:sync(),
    PA = erpc:call(A, erlang, whereis, [cluster_probe]),
    PB = erpc:call(B, erlang, whereis, [cluster_probe]),
    wait(fun() -> global:whereis_name({initial, A}) =:= PA end),
    wait(fun() -> erpc:call(B, global, whereis_name, [{initial, A}]) =:= PA end),
    ok = global:unregister_name({initial, A}),
    yes = erpc:call(A, global, register_name, [cluster_name, PA]),
    ok = global:sync(),
    wait(fun() -> erpc:call(B, global, whereis_name, [cluster_name]) =:= PA end),
    PA = global:whereis_name(cluster_name),
    PA = global:send(cluster_name, {ping, self()}),
    receive pong -> ok after 5000 -> error(send_timeout) end,
    ok = erpc:call(A, pg, join, [cluster_scope, scoped_group, PA]),
    wait(fun() -> pg:get_members(cluster_scope, scoped_group) =:= [PA] end),
    ok = erpc:call(A, pg, join, [cluster_group, PA]),
    ok = erpc:call(B, pg, join, [cluster_group, PB]),
    ok = pg:join(cluster_group, self()),
    wait(fun() -> lists:sort(pg:get_members(cluster_group)) =:= lists:sort([PA, PB, self()]) end),
    wait(fun() -> lists:sort(erpc:call(A, pg, get_members, [cluster_group])) =:= lists:sort([PA, PB, self()]) end),
    wait(fun() -> lists:sort(erpc:call(B, pg, get_members, [cluster_group])) =:= lists:sort([PA, PB, self()]) end),
    Self = self(),
    [Self] = pg:get_local_members(cluster_group),
    [PA] = erpc:call(A, pg, get_local_members, [cluster_group]),
    [PB] = erpc:call(B, pg, get_local_members, [cluster_group]),
    ok = erpc:call(B, pg, leave, [cluster_group, PB]),
    wait(fun() -> lists:sort(pg:get_members(cluster_group)) =:= lists:sort([PA, self()]) end),
    ok = global:unregister_name(cluster_name),
    wait(fun() -> erpc:call(A, global, whereis_name, [cluster_name]) =:= undefined end),
    yes = erpc:call(A, global, register_name, [cluster_name, PA, fun global:random_exit_name/3]),
    wait(fun() -> global:whereis_name(cluster_name) =:= PA end),
    true = monitor_node(A, true),
    PA ! stop,
    receive {nodedown, A} -> ok after 5000 -> error(nodedown_timeout) end,
    wait(fun() -> global:whereis_name(cluster_name) =:= undefined end),
    wait(fun() -> pg:get_members(cluster_group) =:= [self()] end),
    wait(fun() -> pg:get_members(cluster_scope, scoped_group) =:= [] end),
    PB ! stop,
    io:format("cluster: ok~n").

wait(F) -> wait(F, 100).
wait(F, N) when N > 0 ->
    case F() of
        true -> ok;
        false -> timer:sleep(50), wait(F, N - 1)
    end;
wait(_, 0) -> error(timeout).
