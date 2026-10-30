%% Links between a process of a server node and a process of a client
%% node, in both directions, with and without trap_exit, and with the exit
%% reasons normal, kill, and a reason that is not immediate (bif.c:263-394,
%% 1295-1348, dist.c:2224-2339, 2745-2805, erl_process.c:14101-14147).
-module(dist_link_probe).
-export([server/0, client/1]).

%% Print the node name, and make a process for each request of a client.
server() ->
    io:format("~s~n", [node()]),
    register(dlink, self()),
    serve().

serve() ->
    receive
        {victim, Trap, Back, From} ->
            V = spawn(fun() ->
                              process_flag(trap_exit, Trap),
                              case Back of
                                  none -> ok;
                                  _ -> true = link(Back)
                              end,
                              From ! {victim, self()},
                              victim(From)
                      end),
            spawn(fun() ->
                          R = monitor(process, V),
                          receive {'DOWN', R, _, _, W} -> From ! {down, V, W} end
                  end),
            serve()
    end.

victim(From) ->
    receive
        {exit, Why} -> exit(Why);
        {unlink, Pid} ->
            true = unlink(Pid),
            From ! {unlinked, self()},
            victim(From);
        links -> From ! {links, self(), element(2, process_info(self(), links))}, victim(From);
        Msg -> From ! {got, self(), Msg}, victim(From)
    end.

%% Run each case with the server node Server and print the results.
client(Server) ->
    true = net_kernel:connect_node(Server),
    Cases = [{Dir, Trap, Why} || Dir <- [to, from], Trap <- [false, true],
                                 Why <- [normal, kill, {boom}]],
    io:format("~p~n", [[run(Server, Case) || Case <- Cases ++ [unlink_to, unlink_from]]]).

%% A local process that forwards its messages to the caller.
local(Trap, Body) ->
    Main = self(),
    Pid = spawn(fun() ->
                        process_flag(trap_exit, Trap),
                        Body(),
                        Main ! {ready, self()},
                        forward(Main)
                end),
    receive {ready, Pid} -> ok end,
    {Pid, monitor(process, Pid)}.

forward(Main) ->
    receive
        {exit, Why} -> exit(Why);
        {link, Pid} -> true = link(Pid), Main ! {linked, self()}, forward(Main);
        {unlink, Pid} -> true = unlink(Pid), Main ! {unlinked, self()}, forward(Main);
        links -> Main ! {links, self(), element(2, process_info(self(), links))}, forward(Main);
        Msg -> Main ! {got, self(), Msg}, forward(Main)
    end.

victim(Server, Trap, Back) ->
    {dlink, Server} ! {victim, Trap, Back, self()},
    receive {victim, V} -> V end.

%% Wait for the result of the link exit signal: the EXIT message, or the
%% exit of the linked process; a process that ignores the signal answers
%% a ping.
outcome(Pid, _, false, normal) ->
    Pid ! ping,
    receive {got, Pid, ping} -> alive after 10000 -> timeout end;
outcome(Pid, _, true, _) ->
    receive {got, Pid, {'EXIT', _, W}} -> {'EXIT', W} after 10000 -> timeout end;
outcome(Pid, Ref, false, _) ->
    receive
        {'DOWN', Ref, _, _, W} -> {down, W};
        {down, Pid, W} -> {down, W}
    after 10000 -> timeout
    end.

run(Server, {to, Trap, Why}) ->
    %% The client process links to the server process.
    V = victim(Server, false, none),
    {C, Ref} = local(Trap, fun() -> ok end),
    C ! {link, V},
    receive {linked, C} -> ok end,
    V ! {exit, Why},
    {to, Trap, Why, outcome(C, Ref, Trap, Why)};
run(Server, {from, Trap, Why}) ->
    %% The server process links to the client process.
    {C, _} = local(false, fun() -> ok end),
    V = victim(Server, Trap, C),
    C ! {exit, Why},
    {from, Trap, Why, outcome(V, none, Trap, Why)};
run(Server, unlink_to) ->
    V = victim(Server, false, none),
    {C, _} = local(false, fun() -> ok end),
    C ! {link, V},
    receive {linked, C} -> ok end,
    C ! {unlink, V},
    receive {unlinked, C} -> ok end,
    V ! {exit, boom},
    Down = receive {down, V, W} -> W end,
    C ! links,
    {unlink_to, Down, receive {links, C, L} -> L end};
run(Server, unlink_from) ->
    {C, _} = local(false, fun() -> ok end),
    V = victim(Server, false, C),
    V ! {unlink, C},
    receive {unlinked, V} -> ok end,
    V ! links,
    Links = receive {links, V, L} -> L end,
    V ! {exit, boom},
    Down = receive {down, V, W} -> W end,
    C ! ping,
    {unlink_from, Links, Down, receive {got, C, ping} -> alive end}.
