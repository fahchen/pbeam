-module(setnode_probe).
-export([main/0, stop/0, remote_monitor/0]).

%% Give the local node a name and a creation with erlang:setnode/2
%% (dist.c:4978-5071) and write what reads them: node/0,1, is_alive/0,
%% erts_internal:get_creation/0, system_info(creation), nodes/1, the
%% external term format (external.c:2967-2996, 3123-3129), the text, the
%% term order, names with the node, and ETS.
main() ->
    T = fun(F) -> try F() catch C:E -> {C, E} end end,
    Before = [node(), is_alive(), node(self()), erts_internal:get_creation(),
              erlang:system_info(creation), nodes(this)],
    NoKernel = T(fun() -> erlang:setnode(a@host, 7) end),
    true = register(net_kernel, self()),
    Bad = [T(fun() -> erlang:setnode(N, C) end)
           || {N, C} <- [{nonode@nohost, 7}, {abc, 7}, {'a@b@c', 7}, {'@b', 7},
                         {'a@', 7}, {'a.b@c', 7}, {"a@b", 7}, {a@host, -1},
                         {a@host, 1 bsl 59}, {a@host, 1.0}, {a@host, x}]],
    Old = binary_to_term(<<131, 88, 100, 6:16, "a@host", 9:32, 1:32, 3:32>>),
    Other = binary_to_term(<<131, 88, 100, 6:16, "m@host", 9:32, 1:32, 3:32>>),
    Nonode = binary_to_term(<<131, 88, 100, 13:16, "nonode@nohost", 9:32, 1:32, 0:32>>),
    Set = erlang:setnode(a@host, (1 bsl 32) + 1),
    Again = [T(fun() -> erlang:setnode(N, C) end) || {N, C} <- [{b@host, 7}, {a@host, 9}]],
    Ref = make_ref(),
    Port = open_port({spawn, "cat"}, []),
    New = binary_to_term(<<131, 88, 100, 6:16, "a@host", 9:32, 1:32, 4:32>>),
    Nil = T(fun() -> binary_to_term(<<131, 88, 106, 9:32, 1:32, 5:32>>) end),
    Tab = ets:new(t, []),
    ets:insert(Tab, {self()}),
    Name = probe_name,
    true = register(Name, spawn(fun() -> receive {From, M} -> From ! {got, M} end end)),
    {Name, a@host} ! {self(), hello},
    Got = receive {got, M} -> M after 1000 -> timeout end,
    {_, MRef} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', MRef, _, _, _} -> ok end,
    Mon = monitor(process, {nobody, a@host}),
    Down = receive {'DOWN', Mon, _, Item, Reason} -> {Item, Reason} after 1000 -> timeout end,
    _ = erlang:system_flag(system_logger, self()),
    spawn(fun() -> error(boom) end),
    Log = receive {log, error, Format, [_, LogNode, _], _} -> {Format, LogNode} after 1000 -> timeout end,
    io:format("~p~n", [[
        Before, NoKernel, Bad, Set, Again,
        [node(), is_alive(), node(self()), node(Ref), node(Port),
         erts_internal:get_creation(), erlang:system_info(creation),
         nodes(this)],
        [mask(term_to_binary(I)) || I <- [self(), Ref, Port]],
        [mask(term_to_binary(I, [local])) || I <- [self()]],
        [{is_pid(I), node(I), lists:flatten(io_lib:format("~p", [I]))} || I <- [Old, Nonode]],
        node(Other),
        [New, Nil],
        [mask(term_to_binary(I)) || I <- [Old, Nonode]],
        [case I of Other -> other; Old -> old; Nonode -> nonode; _ -> self end
         || I <- lists:sort([Other, Old, Nonode, self()])],
        [lists:sort([Other, self()]) =:= [Other, self()], self() < Other, Old < self()],
        ets:info(Tab, node), ets:select(Tab, [{{'_'}, [], [{node}]}]),
        Got, Down, Log
    ]]).

%% Give the local node its name back when net_kernel exits
%% (erl_process.c:14471-14506, dist.c:922-1117, 853-872).
stop() ->
    Self = self(),
    Kernel = spawn(fun() -> true = register(net_kernel, self()), Self ! ready,
                            receive stop -> exit(stopped) end end),
    receive ready -> ok end,
    true = erlang:setnode(b@host, (1 bsl 59) - 1),
    Alive = [node(), is_alive(), erts_internal:get_creation()],
    MRef = monitor(process, Kernel),
    Kernel ! stop,
    receive {'DOWN', MRef, _, _, _} -> ok end,
    Down = wait(nonode@nohost, 1000),
    Back = [node(), is_alive(), erts_internal:get_creation(), erlang:system_info(creation),
            mask(term_to_binary(self()))],
    true = register(net_kernel, self()),
    Next = erlang:setnode(c@host, 4),
    io:format("~p~n", [[Alive, Down, Back, Next, node(), erts_internal:get_creation()]]).

%% Monitor a name of another node on an alive node (bif.c:875-893).
remote_monitor() ->
    true = register(net_kernel, self()),
    true = erlang:setnode(a@host, 4),
    monitor(process, {nobody, b@host}).

%% Wait until node/0 gives `Node`: ERTS gives the name back in the aux
%% work of a scheduler after the exit (dist.c:880-883).
wait(Node, 0) -> {timeout, Node, node()};
wait(Node, N) ->
    case node() of
        Node -> ok;
        _ -> receive after 1 -> wait(Node, N - 1) end
    end.

%% Replace the number and the serial of a pid or a port, and the numbers of
%% a reference, with 0, so that the output does not depend on the run.
mask(<<131, 121, _:32, 88, 106, _:64, C:32>>) -> <<131, 121, 0:32, 88, 106, 0:64, C:32>>;
mask(<<131, Tag, 119, L, Node:L/binary, _:64, C:32>>) when Tag =:= 88; Tag =:= 120 ->
    <<131, Tag, 119, L, Node/binary, 0:64, C:32>>;
mask(<<131, 90, N:16, 119, L, Node:L/binary, C:32, _/binary>>) ->
    <<131, 90, N:16, 119, L, Node/binary, C:32>>.
