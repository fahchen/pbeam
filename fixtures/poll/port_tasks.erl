-module(port_tasks).
-export([main/0, owner/0, holder/0]).

%% The owner root opens a tcp_inet socket and a spawn port, gives the
%% socket to the holder root of the other scheduler, and waits. The holder
%% starts a receive on the socket, which adds the socket fd to the poll set
%% of the waiting scheduler, and then makes the client send. The poll of
%% the owner scheduler must wake for the new fd, as erts_poll_control wakes
%% the poller (erl_poll.c:1447-1448), or the receive times out.
%%
%% main/0 runs both roots on the pinned ERTS, for erl +S 2.
main() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {holder, holder()} end),
    ok = owner(),
    receive {holder, Result} -> Result end.

owner() ->
    Holder = holder_pid(100000),
    {ok, L} = prim_inet:open(tcp, inet, stream),
    ok = prim_inet:setopts(L, [binary, {active, false}]),
    {ok, _} = prim_inet:bind(L, {127, 0, 0, 1}, 0),
    ok = prim_inet:listen(L, 5),
    {ok, {_, Number}} = prim_inet:sockname(L),
    Script = "read -r go; exec 3<>/dev/tcp/127.0.0.1/" ++ integer_to_list(Number)
        ++ "; read -r go; printf hi >&3; read -r stop",
    Client = open_port({spawn_executable, "/bin/bash"}, [binary, {args, ["-c", Script]}]),
    Holder ! {listen, self(), L, Client},
    receive done -> ok after 10000 -> timeout end.

%% Each request of the holder waits in the poll of the owner scheduler: the
%% accept for the listen socket, and the receive for the new socket, which
%% the owner scheduler accepted for the holder (io.c:837-844).
holder() ->
    true = register(port_tasks_holder, self()),
    {Owner, L, Client} = receive {listen, From, Listen, Port} -> {From, Listen, Port} end,
    Accepted = request(Client, L, fun() -> prim_inet:async_accept(L, 5000) end),
    {ok, S} = Accepted,
    {connected, Connected} = erlang:port_info(S, connected),
    Received = request(Client, S, fun() -> prim_inet:async_recv(S, 0, 5000) end),
    Owner ! done,
    {element(1, Accepted), Connected =:= self(), Received}.

%% The wait lets the owner scheduler start its idle wait before the request.
request(Client, Socket, Request) ->
    receive after 100 -> ok end,
    {ok, Ref} = Request(),
    true = port_command(Client, <<"go\n">>),
    receive {inet_async, Socket, Ref, Status} -> Status after 10000 -> missing end.

holder_pid(0) -> error(no_holder);
holder_pid(N) ->
    case whereis(port_tasks_holder) of
        undefined -> erlang:yield(), holder_pid(N - 1);
        Pid -> Pid
    end.
