-module(check_time_probe).
-export([short_turns/0]).

%% Start a timer of 1 ms, then exchange messages with a peer in short turns
%% until the timer message arrives. The scheduler never sleeps, so it reads
%% the clock only after its processes used ERTS_CHECK_TIME_REDS (4000)
%% reductions. Return whether the two processes used at least 4000
%% reductions before the timer message arrived.
short_turns() ->
    Self = self(),
    Peer = spawn(fun() -> peer(Self) end),
    {reductions, A0} = process_info(self(), reductions),
    {reductions, B0} = process_info(Peer, reductions),
    erlang:send_after(1, self(), t),
    rounds(Peer),
    {reductions, A1} = process_info(self(), reductions),
    {reductions, B1} = process_info(Peer, reductions),
    exit(Peer, kill),
    (A1 - A0) + (B1 - B0) >= 4000.

rounds(Peer) ->
    Peer ! ping,
    receive
        t ->
            receive pong -> ok end;
        pong ->
            rounds(Peer)
    end.

peer(Parent) ->
    receive
        ping ->
            Parent ! pong,
            peer(Parent)
    end.
