-module(tcp_active).
-export([main/1, active/1, closed/1, passive_closed/1, links/0, link_orders/0, pid_port_links/0, unsupported/2, unsupported_item/0]).

%% Delivers the data of an accepted socket in active mode, as
%% inet:setopts/2 calls prim_inet:setopts/2, and gives the socket to a
%% second process, as gen_tcp:controlling_process/2 calls
%% inet:tcp_controlling_process/2 (inet.erl:4595-4660). The messages are
%% {tcp, S, Data} and {tcp_closed, S} (inet_drv.c:3987-4077).
%%
%% The probe writes "Step Command" to the file Path when the client of
%% the test must act, and waits until the client writes Step to the file
%% Path.ack. The probe traps exits, as the process of `erl -eval` does.

%% The owner reads with {active, once} twice, moves the socket and a
%% queued message to a handler, and the handler reads with {active, once}
%% until the client closes.
main(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    true = inet_db:register_socket(S, inet_tcp),
    step(Path, 2, "send abc"),
    First = [prim_inet:setopts(S, [{active, once}]), message(S), prim_inet:getopts(S, [active])],
    step(Path, 3, "send def"),
    %% The data comes in the reply of setopts, and the message stays in
    %% the queue of the owner.
    ok = prim_inet:setopts(S, [{active, once}]),
    Once = [prim_inet:getopts(S, [active]), prim_inet:setopts(S, [{active, once}]),
            prim_inet:getopts(S, [active])],
    Parent = self(),
    Handler = spawn(fun() -> process_flag(trap_exit, true), handler(Parent) end),
    Other = spawn(fun() -> Parent ! {self(), gen_tcp:controlling_process(S, self())} end),
    NotOwner = receive {Other, NotOwnerResult} -> NotOwnerResult end,
    Before = [erlang:port_info(S, connected) =:= {connected, self()},
              erlang:port_info(S, links) =:= {links, [self()]}],
    Moved = gen_tcp:controlling_process(S, Handler),
    After = [erlang:port_info(S, connected) =:= {connected, Handler},
             erlang:port_info(S, links) =:= {links, [Handler]},
             gen_tcp:controlling_process(S, Handler), prim_inet:getopts(S, [active])],
    Handler ! {go, S},
    Queued = receive {handler, QueuedMessage} -> QueuedMessage after 3000 -> missing end,
    step(Path, 4, "send ghi"),
    Data = receive {handler, DataMessage} -> DataMessage after 3000 -> missing end,
    step(Path, 5, "close"),
    Closed = [receive {handler, ClosedMessage} -> ClosedMessage after 3000 -> missing end,
              receive {handler, ExitMessage} -> ExitMessage after 3000 -> missing end],
    End = [erlang:port_info(S, connected), gen_tcp:controlling_process(S, Handler)],
    ok = prim_inet:close(L),
    [First, Once, NotOwner, Before, Moved, After, Queued, Data, Closed, End, flush()].

%% {active, true} delivers each read until {active, false}.
active(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    step(Path, 2, "send x"),
    Set = prim_inet:setopts(S, [{active, true}]),
    X = message(S),
    step(Path, 3, "send y"),
    Y = message(S),
    Get = prim_inet:getopts(S, [active]),
    ok = prim_inet:setopts(S, [{active, false}]),
    step(Path, 4, "send z"),
    Z = [message0(S), prim_inet:recv(S, 0, 1000)],
    ok = prim_inet:setopts(S, [list, {active, true}]),
    step(Path, 5, "send w"),
    W = message(S),
    ok = prim_inet:close(L),
    [Set, X, Y, Get, Z, W, flush()].

%% The client closes before {active, once}: the setopts reads the end of
%% file, sends {tcp_closed, S}, and the port exits with the reason normal.
closed(Path) ->
    process_flag(trap_exit, true),
    {L, S} = connection(Path),
    step(Path, 2, "send bye"),
    step(Path, 3, "close"),
    Set = prim_inet:setopts(S, [{active, once}]),
    Data = message(S),
    Again = prim_inet:setopts(S, [{active, once}]),
    Messages = flush(),
    ok = prim_inet:close(L),
    [Set, Data, Again, Messages, erlang:port_info(S, connected)].

%% A passive recv reads the end of file and closes the socket. A later
%% {active, once} or {active, true} sends {tcp_closed, S}, skips the next
%% options, and the port exits with the reason normal
%% (inet_drv.c:7001-7011).
passive_closed(Path) ->
    process_flag(trap_exit, true),
    %% prim_inet:setopts/2 puts active last, so the first SETOPTS of
    %% {active, once} and then {tos, 1} goes to the port directly
    %% (prim_inet.erl:1028, 2972-2975).
    SetOnce = fun(S) -> erlang:port_control(S, 16#03f1a307, [22, 0, 0, 0, 2, 9, 0, 0, 0, 1]) end,
    SetTrue = fun(S) -> prim_inet:setopts(S, [{active, true}]) end,
    [ended(Path, 1, SetOnce, once), ended(Path, 3, SetTrue, true)].

ended(Path, Step, Setopts, Active) ->
    {L, S} = connection(Path, Step),
    step(Path, Step + 1, "close"),
    Recv = prim_inet:recv(S, 0, 1000),
    Set = Setopts(S),
    Closed = message(S),
    Again = prim_inet:setopts(S, [{active, Active}]),
    ok = prim_inet:close(L),
    [Recv, Set, Closed, Again, erlang:port_info(S, connected), flush()].

%% link/1, unlink/1, erlang:port_connect/2, and erlang:port_info/2 with a
%% port. Each read of the links item is of a port with zero or one link
%% (see link_orders/0).
links() ->
    process_flag(trap_exit, true),
    {ok, S} = prim_inet:open(tcp, inet, stream),
    Parent = self(),
    Linker = spawn(fun() -> Parent ! {self(), link(S)}, receive stop -> ok end end),
    Linked = receive {Linker, LinkResult} -> LinkResult end,
    Unlinked = [unlink(S), unlink(S)],
    One = erlang:port_info(S, links) =:= {links, [Linker]},
    Relinked = link(S),
    %% A normal exit of a linked process that is not the owner leaves the
    %% port open.
    Linker ! stop,
    Down = monitor(process, Linker),
    receive {'DOWN', Down, process, Linker, normal} -> ok end,
    Open = erlang:port_info(S, links) =:= {links, [Parent]},
    DeadConnect = c(fun() -> erlang:port_connect(S, Linker) end),
    Live = spawn(fun() -> receive stop -> ok end end),
    Connected = [erlang:port_connect(S, self()), erlang:port_connect(S, Live)],
    Owner = [erlang:port_info(S, connected) =:= {connected, Live}],
    Infos = [erlang:port_info(S, id) =:= {id, element(2, erlang:port_info(S, id))},
             erlang:port_info(S, name), erlang:port_info(no_port, connected)],
    Errors = [c(fun() -> erlang:port_info(S, bad_item) end),
              c(fun() -> erlang:port_info(S, "connected") end),
              c(fun() -> erlang:port_info(1, connected) end),
              c(fun() -> erlang:port_connect(S, self) end),
              c(fun() -> erlang:port_connect(no_port, self()) end),
              c(fun() -> link(no_port) end)],
    %% An exit of a linked process that is not normal closes the port.
    Killer = spawn(fun() -> link(S), Parent ! linked, exit(kill) end),
    receive linked -> ok end,
    Exits = [receive {'EXIT', S, Reason} -> Reason after 3000 -> missing end,
             erlang:port_info(S, connected)],
    Dead = [c(fun() -> link(S) end), receive {'EXIT', S, DeadReason} -> DeadReason after 0 -> none end,
            c(fun() -> process_flag(trap_exit, false), link(S) end)],
    process_flag(trap_exit, true),
    _ = Killer,
    %% A link signal to a process that a kill ends first closes the port
    %% with noproc (erl_proc_sig_queue.c:7363-7368).
    {ok, S2} = prim_inet:open(tcp, inet, stream),
    Victim = spawn(fun() -> receive stop -> ok end end),
    exit(Victim, kill),
    Killed = [erlang:port_connect(S2, Victim),
              receive {'EXIT', S2, KilledReason} -> KilledReason after 3000 -> missing end],
    [Killed, Linked, Unlinked, One, Relinked, Open, DeadConnect, Connected, Owner, Infos, Errors, Exits, Dead,
     flush()].

%% The links item of a port with several links, after each link and unlink
%% of the members in a different order, the links item of
%% erlang:port_info/1, and the order of the link exit signals of a port
%% that closes. Each list gives the position of the linked process in the
%% spawn order of the run, with the owner first. The ERTS link tree gives
%% the order: `ml_cmp_keys` orders the tree (erl_monitor_link.c:91-233),
%% `foreach_unordered__` walks it (erl_rbtree.h:1343-1458), and
%% `erts_bld_port_info` conses each entry to the front
%% (erl_bif_info.c:3676-3696).
link_orders() ->
    process_flag(trap_exit, true),
    {ok, S1} = prim_inet:open(tcp, inet, stream),
    {ok, S2} = prim_inet:open(tcp, inet, stream),
    Owner = self(),
    Members = [Owner | [spawn(fun() -> member(Owner) end) || _ <- lists:seq(1, 6)]],
    [orders(S1, Members, [{link, 2}, {link, 3}, {link, 4}, {link, 5}, {unlink, 3}, {link, 6},
                          {unlink, 2}, {link, 3}, {unlink, 5}, {link, 2}, {unlink, 6}]),
     orders(S2, Members, [{link, 6}, {link, 5}, {link, 4}, {unlink, 6}, {link, 3}, {link, 2},
                          {link, 6}, {unlink, 4}, {unlink, 5}, {link, 4}, {unlink, 2}]),
     whole_info_order(S1, Members),
     exit_order(S1, Members)].

%% The links of a process to pids and ports in the order of the pinned ERTS
%% tree. ERTS compares a local pid and a local port by their tags first
%% (`ml_cmp_keys`, erl_monitor_link.c:146-150), so each pid comes before
%% each port, whatever their numbers are. The process links members and
%% ports in turn, gives its links item (erl_bif_info.c:1757-1775), and
%% exits; each member reports its link exit signal, in the order of the
%% walk of the tree (erl_process.c:14545-14551).
pid_port_links() ->
    process_flag(trap_exit, true),
    Owner = self(),
    Members = [spawn(fun() -> member(Owner) end) || _ <- lists:seq(1, 4)],
    Ports = [element(2, prim_inet:open(tcp, inet, stream)) || _ <- lists:seq(1, 4)],
    Linker = spawn(fun() -> linker(Owner) end),
    Linked = lists:append([[M, P] || {M, P} <- lists:zip(Members, Ports)]),
    [link_from(Linker, Member) || Member <- Linked],
    Linker ! {links, self()},
    Info = receive {Linker, Links} -> [position(Link, Linked) || Link <- Links] end,
    Linker ! stop,
    Exits = [receive {gone, Member} -> position(Member, Linked) end || _ <- Members],
    [Info, Exits].

%% Make `Linker` link `Member`: a member links `Linker` when the probe asks,
%% and `Linker` links a port itself.
link_from(Linker, Member) when is_pid(Member) ->
    Member ! {link, Linker, self()},
    receive {Member, ok} -> ok end;
link_from(Linker, Port) ->
    Linker ! {link, Port, self()},
    receive {Linker, ok} -> ok end.

linker(Owner) ->
    receive
        {link, Port, From} ->
            link(Port),
            From ! {self(), ok},
            linker(Owner);
        {links, From} ->
            {links, Links} = process_info(self(), links),
            From ! {self(), Links},
            linker(Owner);
        stop ->
            exit(boom)
    end.

%% The order of the link exit signals of a port that closes
%% (`erts_deliver_port_exit`, io.c:3869-3872, 3922-3925; `link_port_exit`,
%% io.c:3808-3819). Each linked member traps exits, reports its exit
%% signal, and stops.
exit_order(Port, Members) ->
    {links, Links} = erlang:port_info(Port, links),
    Linked = length(Links) - 1,
    erlang:port_close(Port),
    %% The probe is the owner, so the port sends it an exit signal too.
    receive {'EXIT', Port, _} -> ok end,
    [receive {gone, Member} -> position(Member, Members) end || _ <- lists:seq(1, Linked)].

%% The links item of erlang:port_info/1 gives the item of
%% erlang:port_info/2.
whole_info_order(Port, Members) ->
    {links, Links} = lists:keyfind(links, 1, erlang:port_info(Port)),
    [position(Link, Members) || Link <- Links].

%% The links order of the port before the operations and after each one.
orders(Port, Members, Ops) ->
    [order(Port, Members) | run(Ops, Port, Members)].

run([], _Port, _Members) ->
    [];
run([{Op, Index} | Rest], Port, Members) ->
    Member = lists:nth(Index, Members),
    Member ! {Op, Port, self()},
    receive {Member, ok} -> ok end,
    [order(Port, Members) | run(Rest, Port, Members)].

order(Port, Members) ->
    {links, Links} = erlang:port_info(Port, links),
    [position(Link, Members) || Link <- Links].

position(Pid, Members) ->
    position(Pid, Members, 1).

position(Pid, [Pid | _], Index) ->
    Index;
position(Pid, [_ | Rest], Index) ->
    position(Pid, Rest, Index + 1).

%% A process that links and unlinks a port when the probe asks, and reports
%% the link exit signal of a port that closes.
member(Owner) ->
    process_flag(trap_exit, true),
    receive
        {link, Port, From} ->
            link(Port),
            From ! {self(), ok},
            member(Owner);
        {unlink, Port, From} ->
            unlink(Port),
            From ! {self(), ok},
            member(Owner);
        {'EXIT', _Port, _Reason} ->
            Owner ! {gone, self()}
    end.

%% Each part that PBeam does not run yet.
unsupported(Path, Case) ->
    process_flag(trap_exit, true),
    {_, S} = connection(Path),
    case Case of
        count -> prim_inet:setopts(S, [{active, 2}])
    end.

%% An item of erlang:port_info/2 that PBeam does not give yet.
unsupported_item() ->
    {ok, S} = prim_inet:open(tcp, inet, stream),
    erlang:port_info(S, memory).

handler(Parent) ->
    receive
        {go, S} ->
            Parent ! {handler, message(S)},
            ok = prim_inet:setopts(S, [{active, once}]),
            Parent ! {handler, message(S)},
            ok = prim_inet:setopts(S, [{active, once}]),
            Parent ! {handler, message(S)},
            %% The port unlinks its owner before its exit (io.c:7238-7239).
            Parent ! {handler, receive Exit -> Exit after 500 -> none end}
    end.

message(S) ->
    receive
        {tcp, S, Data} -> {tcp, Data};
        {tcp_closed, S} -> tcp_closed;
        {tcp_error, S, Reason} -> {tcp_error, Reason}
    after 3000 -> missing
    end.

message0(S) ->
    receive {tcp, S, Data} -> {tcp, Data} after 0 -> none end.

c(F) ->
    try F() catch Class:Reason -> {Class, Reason} end.

connection(Path) ->
    connection(Path, 1).

connection(Path, Step) ->
    {ok, L} = prim_inet:open(tcp, inet, stream),
    ok = prim_inet:setopts(L, [binary, {active, false}]),
    {ok, _} = prim_inet:bind(L, {127, 0, 0, 1}, 0),
    ok = prim_inet:listen(L, 5),
    {ok, {_, Port}} = prim_inet:sockname(L),
    step(Path, Step, ["connect ", integer_to_list(Port)]),
    {ok, S} = prim_inet:accept(L, 5000, []),
    {L, S}.

%% Ask the client of the test for Command, and wait until it has run it.
step(Path, Step, Command) ->
    Script = "printf '%s %s\\n' \"$1\" \"$2\" > \"$3.tmp\" && mv \"$3.tmp\" \"$3\" && "
             "until [ \"$(cat \"$3.ack\" 2>/dev/null)\" = \"$1\" ]; do sleep 0.01; done",
    Shell = open_port({spawn_executable, "/bin/sh"},
                      [binary, exit_status,
                       {args, ["-c", Script, "sh", integer_to_list(Step),
                               lists:flatten(Command), Path]}]),
    receive {Shell, {exit_status, 0}} -> ok end,
    receive {'EXIT', Shell, normal} -> ok end.

flush() ->
    receive
        Message when is_tuple(Message) ->
            [[if is_port(E) -> port; is_pid(E) -> pid; is_reference(E) -> ref; true -> E end
              || E <- tuple_to_list(Message)] | flush()]
    after 0 -> []
    end.
