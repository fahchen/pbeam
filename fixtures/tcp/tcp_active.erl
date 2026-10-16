-module(tcp_active).
-export([main/1, active/1, closed/1, links/0, unsupported/2, unsupported_item/0]).

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

%% link/1, unlink/1, erlang:port_connect/2, and erlang:port_info/2 with a
%% port.
links() ->
    process_flag(trap_exit, true),
    {ok, S} = prim_inet:open(tcp, inet, stream),
    Parent = self(),
    Linker = spawn(fun() -> Parent ! {self(), link(S)}, receive stop -> ok end end),
    Linked = receive {Linker, LinkResult} -> LinkResult end,
    Two = erlang:port_info(S, links) =:= {links, lists:sort([Parent, Linker])}
        orelse erlang:port_info(S, links),
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
    Connected = erlang:port_connect(S, Live),
    Owner = [erlang:port_info(S, connected) =:= {connected, Live},
             erlang:port_info(S, links) =:= {links, lists:sort([Parent, Live])}
             orelse erlang:port_info(S, links)],
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
    [Linked, Two, Unlinked, One, Relinked, Open, DeadConnect, Connected, Owner, Infos, Errors, Exits, Dead,
     flush()].

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
    {ok, L} = prim_inet:open(tcp, inet, stream),
    ok = prim_inet:setopts(L, [binary, {active, false}]),
    {ok, _} = prim_inet:bind(L, {127, 0, 0, 1}, 0),
    ok = prim_inet:listen(L, 5),
    {ok, {_, Port}} = prim_inet:sockname(L),
    step(Path, 1, ["connect ", integer_to_list(Port)]),
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
