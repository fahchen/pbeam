%% The distribution controller sees the order of signals queued before setup.
-module(dist_pending_order_probe).
-export([main/0]).

main() ->
    Node = pending_order@nohost,
    Name = atom_to_binary(Node),
    Pid = binary_to_term(<<131, 88, 118, (byte_size(Name)):16, Name/binary, 1:32, 0:32, 1:32>>),
    Handle = erts_internal:new_connection(Node),
    {sink, Node} ! first,
    true = link(Pid),
    _ = monitor(process, Pid),
    _ = spawn_request(Node, erlang, node, [], []),
    true = exit(Pid, boom),
    Flags = element(3, erts_internal:get_dflags()) bor 16#28 bor (1 bsl 32),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    %% OTP queues the five controls in send, link, monitor, spawn, exit order
    %% (dist.c:3595-3607, 3646-3683; dist.h:143-171). Read the controller
    %% packets: a spawned process runs independently, so mailbox arrival from
    %% that process cannot establish the order of the spawn request packet.
    Read = fun() ->
                   Packet = iolist_to_binary(erlang:dist_ctrl_get_data(Handle)),
                   <<112, Control/binary>> = Packet,
                   {Tuple, _} = binary_to_term(Control, [used]),
                   element(1, Tuple)
           end,
    Order = [Read(), Read(), Read(), Read(), Read()],
    none = erlang:dist_ctrl_get_data(Handle),
    io:format("~w~n", [Order]).
