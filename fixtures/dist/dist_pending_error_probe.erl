%% A legal alias remains a reference after its owner exits.
-module(dist_pending_error_probe).
-export([main/1, deliver/0]).

main(Operation) ->
    Alias = dead_alias(),
    Node = pending_error@nohost,
    Pid = remote_pid(Node),
    Handle = erts_internal:new_connection(Node),
    {ok, _, _, Before} = erlang:dist_get_stat(Handle),
    case Operation of
        send -> Pid ! Alias;
        exit_signal -> true = exit(Pid, Alias);
        link_exit ->
            {Child, ChildMonitor} = spawn_monitor(fun() -> true = link(Pid), exit(Alias) end),
            receive {'DOWN', ChildMonitor, process, Child, Alias} -> ok end
    end,
    {ok, _, _, After} = erlang:dist_get_stat(Handle),
    io:format("~w~n", [{ok, After - Before}]).

%% The External term format row and #1619 record the missing owner at delivery.
deliver() ->
    Alias = dead_alias(),
    Node = pending_delivery@nohost,
    Handle = erts_internal:new_connection(Node),
    remote_pid(Node) ! Alias,
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    io:format("ok~n").

dead_alias() ->
    Parent = self(),
    {Owner, OwnerMonitor} = spawn_monitor(fun() -> Parent ! {self(), alias()} end),
    receive {'DOWN', OwnerMonitor, process, Owner, _} -> ok end,
    receive {Owner, Alias} -> Alias end.

remote_pid(Node) ->
    Name = atom_to_binary(Node),
    binary_to_term(<<131,88,118,(byte_size(Name)):16,Name/binary,1:32,0:32,1:32>>).
