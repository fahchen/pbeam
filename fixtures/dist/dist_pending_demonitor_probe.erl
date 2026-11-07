%% A PID demonitor is sent on an existing pending connection.
-module(dist_pending_demonitor_probe).
-export([server/0, client/2]).

server() ->
    register(pending_demonitor_receiver, self()),
    io:format("~w~n~w~n", [node(), binary_to_list(term_to_binary(self()))]),
    receive
        {inspect, Watcher} ->
            {monitored_by, Watchers} = process_info(self(), monitored_by),
            Watcher ! {monitored, lists:member(Watcher, Watchers)}
    end,
    receive stop -> ok after 5000 -> ok end.

client(Node, PidBytes) ->
    Pid = binary_to_term(list_to_binary(PidBytes)),
    _ = erts_internal:new_connection(Node),
    Ref = monitor(process, Pid),
    true = demonitor(Ref),
    {pending_demonitor_receiver, Node} ! {inspect, self()},
    true = net_kernel:connect_node(Node),
    Result = receive {monitored, Value} -> Value after 5000 -> timeout end,
    io:format("~w~n", [Result]).
