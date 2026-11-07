%% A legal alias remains a reference after its owner exits.
-module(dist_pending_error_probe).
-export([main/1]).

main(Operation) ->
    Parent = self(),
    {Owner, OwnerMonitor} = spawn_monitor(fun() -> Parent ! {self(), alias()} end),
    receive {'DOWN', OwnerMonitor, process, Owner, _} -> ok end,
    Alias = receive {Owner, A} -> A end,
    Node = pending_error@nohost,
    Name = atom_to_binary(Node),
    Pid = binary_to_term(<<131,88,118,(byte_size(Name)):16,Name/binary,1:32,0:32,1:32>>),
    _ = erts_internal:new_connection(Node),
    case Operation of
        send -> Pid ! Alias;
        exit_signal -> true = exit(Pid, Alias);
        link_exit ->
            {Child, ChildMonitor} = spawn_monitor(fun() -> true = link(Pid), exit(Alias) end),
            receive {'DOWN', ChildMonitor, process, Child, Alias} -> ok end
    end,
    io:format("ok~n").
