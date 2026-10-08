-module(receive_probe).
-export([receives/0, orders/0, gcs/0, start/3, traced/3]).
-export([recv_fetch/3, recv_poll/3, recv_queued/3, recv_skip/3, recv_return/3]).
-export([observe/3, woken/2, queued/2, info/2, named/2, hibernated/2, witness/1]).

%% The receive costs of the pinned ERTS JIT. loop_rec costs 0 when a
%% fetched message is next. At the end of the fetched messages, it fetches
%% the messages that arrived and costs 4, also when no message arrived.
%% loop_rec_end costs 1. loop_rec switches out before the peek when the
%% budget is 0 or less.

%% Two processes a and b run the same loop. Each iteration sends the tag
%% of the process to the collector. The collector returns the run lengths
%% of the tags in arrival order: {Tag, Count} for each run. With one
%% scheduler, each run is one turn of the process.
receives() ->
    [{Loop, race(Loop, 2000)}
     || Loop <- [recv_fetch, recv_poll, recv_queued, recv_skip, recv_return]].

%% Start a and b from a new process, so that the reductions of the caller do
%% not change the order of the two spawns.
race(Loop, N) ->
    spawn(?MODULE, start, [self(), Loop, N]),
    receive First -> collect(2 * N - 1, First, 1) end.

start(Collector, Loop, N) ->
    spawn(?MODULE, Loop, [Collector, a, N]),
    spawn(?MODULE, Loop, [Collector, b, N]).

%% Receive K more tags. Tag is the tag of the current run, and Count is its
%% length so far.
collect(0, Tag, Count) -> [{Tag, Count}];
collect(K, Tag, Count) ->
    receive
        Next when Next =:= Tag -> collect(K - 1, Tag, Count + 1);
        Next -> [{Tag, Count} | collect(K - 1, Next, 1)]
    end.

%% A send to self, then a receive that fetches the message: entry 1,
%% send 1 + 4, fetch 4, send 1 + 4.
recv_fetch(_, _, 0) -> ok;
recv_fetch(P, T, N) ->
    self() ! x,
    receive x -> ok end,
    P ! T,
    recv_fetch(P, T, N - 1).

%% A receive with an empty queue and a timeout of 0: entry 1, fetch 4,
%% send 1 + 4.
recv_poll(_, _, 0) -> ok;
recv_poll(P, T, N) ->
    receive x -> ok after 0 -> ok end,
    P ! T,
    recv_poll(P, T, N - 1).

%% N sends to self, then N receives. The first receive fetches all the
%% messages, and each other receive costs 0: entry 1, send 1 + 4.
recv_queued(P, T, N) ->
    fill(N),
    take(P, T, N).

%% As recv_queued, with a message that each receive skips: entry 1,
%% loop_rec_end 1, send 1 + 4.
recv_skip(P, T, N) ->
    self() ! skip,
    fill(N),
    take(P, T, N).

fill(0) -> ok;
fill(N) ->
    self() ! x,
    fill(N - 1).

take(_, _, 0) -> ok;
take(P, T, N) ->
    receive x -> ok end,
    P ! T,
    take(P, T, N - 1).

%% A return before the receive can leave a budget of 0. Then loop_rec
%% switches out before it fetches, and the fetch costs 4 in the next turn.
recv_return(_, _, 0) -> ok;
recv_return(P, T, N) ->
    post(x),
    nop(),
    receive x -> ok end,
    P ! T,
    recv_return(P, T, N - 1).

post(M) -> self() ! M.

nop() -> ok.

%% The order of the message w of a process W that receives and then burns
%% N reductions, and the messages q1 and q2 of a witness process that runs
%% after W. The witness sends q1, uses its budget, and sends q2 in its next
%% turn. For each kind, W sends w in the turn of the receive with the first
%% N, and in its next turn with the second N. For woken, queued, and
%% hibernated, the two values give the exact cost of the receive. For info
%% and named, the two values are 2 apart: process_info/2 also costs 1 for
%% its item (erl_bif_info.c:1601), which PBeam does not charge yet.
orders() ->
    [{Kind, [order(Kind, N) || N <- Ns]}
     || {Kind, Ns} <- [{woken, [3992, 3993]}, {queued, [3979, 3980]}, {info, [3982, 3984]},
                       {named, [3982, 3984]}, {hibernated, [3984, 3985]}]].

order(Kind, N) ->
    spawn(?MODULE, observe, [self(), Kind, N]),
    receive {order, Order} -> Order end.

observe(Collector, woken, N) ->
    Self = self(),
    W = spawn(?MODULE, woken, [Self, N]),
    receive ready -> ok end,
    W ! go,
    spawn(?MODULE, witness, [Self]),
    Collector ! {order, three()};
observe(Collector, Kind, N) ->
    Self = self(),
    W = spawn(?MODULE, Kind, [Self, N]),
    W ! go,
    sends(W, 10),
    spawn(?MODULE, witness, [Self]),
    Collector ! {order, three()}.

sends(_, 0) -> ok;
sends(W, K) ->
    W ! K,
    sends(W, K - 1).

three() ->
    First = receive M1 -> M1 end,
    Second = receive M2 -> M2 end,
    Third = receive M3 -> M3 end,
    [First, Second, Third].

witness(O) ->
    O ! q1,
    burn(4000),
    O ! q2.

%% W waits at the receive: the fetch costs 4 in the turn that ends. A
%% message wakes W: the fetch costs 4 in the new turn. Then N + 1 entries
%% of burn/1, its return, and the send.
woken(O, N) ->
    O ! ready,
    receive go -> ok end,
    burn(N),
    O ! w.

%% Eleven messages arrived before W runs. Entry 1, the first receive
%% fetches all of them for 4, 11 entries of recv/1 and its return, N + 1
%% entries of burn/1, its return, and the send.
queued(O, N) ->
    receive go -> ok end,
    recv(10),
    burn(N),
    O ! w.

%% As queued/2, but process_info/2 fetches the messages with no cost, so
%% no receive fetches.
info(O, N) ->
    {message_queue_len, 11} = process_info(self(), message_queue_len),
    receive go -> ok end,
    recv(10),
    burn(N),
    O ! w.

%% As info/2, with registered_name. process_info/2 of the caller fetches for
%% each item (erl_bif_info.c:1355-1356).
named(O, N) ->
    [] = process_info(self(), registered_name),
    receive go -> ok end,
    recv(10),
    burn(N),
    O ! w.

%% erlang:hibernate/0 fetches the messages and switches out, because a
%% message is in the queue. In the next turn, no receive fetches.
hibernated(O, N) ->
    erlang:hibernate(),
    receive go -> ok end,
    recv(10),
    burn(N),
    O ! w.

recv(0) -> ok;
recv(N) ->
    receive _ -> recv(N - 1) end.

burn(0) -> ok;
burn(N) -> burn(N - 1).

%% ERTS only: the number of garbage collections in each loop. Each loop
%% must give 0, else a collection changes the reductions on ERTS.
gcs() ->
    [{Loop, gc_count(Loop)}
     || Loop <- [recv_fetch, recv_poll, recv_queued, recv_skip, recv_return]].

gc_count(Loop) ->
    Self = self(),
    {Pid, Ref} = spawn_monitor(?MODULE, traced, [Self, Loop, 2000]),
    erlang:trace(Pid, true, [garbage_collection, {tracer, Self}]),
    Pid ! go,
    gc_events(Ref, 0).

traced(Collector, Loop, N) ->
    receive go -> ok end,
    ?MODULE:Loop(Collector, a, N).

gc_events(Ref, Count) ->
    receive
        {'DOWN', Ref, _, _, normal} -> Count;
        {trace, _, gc_minor_start, _} -> gc_events(Ref, Count + 1);
        {trace, _, gc_major_start, _} -> gc_events(Ref, Count + 1);
        _ -> gc_events(Ref, Count)
    end.
