-module(preempt_probe).
-export([sends/0, calls/0, traps/0, bumps/0, yields/0, reductions/0, exits/0, gcs/0, start/3,
         traced/3, id/1]).
-export([send_op/3, send_ext/3, send_apply/3, send_fun/3, send_only/3, send_apply_only/3]).
-export([observe/2, burn_return/1, send_tag/1]).
-export([call_light/3, call_heavy/3, call_apply/3, call_fun/3, call_only/3,
         call_last/3, call_apply_only/3, call_heavy_only/3, call_guard/3]).
-export([trap_ext/3, trap_apply/3]).
-export([bump_loop/3, yield_op/3, yield_only/3, yield_last/3, yield_apply/3, measured/3, sink/0,
         bump_yield/0]).

%% The reduction total of the calling process. element/2 is a guard BIF,
%% so it costs no reduction.
-define(REDS, element(2, process_info(self(), reductions))).

%% The operations of measured/3.
-define(OPS, [empty, body, light, heavy, send, bump, clamp, yield, yield_apply, apply, 'fun',
              guard]).

%% Two processes a and b run the same busy loop. Each iteration sends the
%% tag of the process to the collector. The collector returns the run
%% lengths of the tags in arrival order: {Tag, Count} for each run. With
%% one scheduler, each run is one turn of the process. Thus the run lengths
%% show the reductions of each operation in the loop.

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant.
id(X) -> X.

%% The loops that send in different ways.
sends() ->
    [{Loop, race(Loop, 2000)}
     || Loop <- [send_op, send_ext, send_apply, send_fun, send_only, send_apply_only]].

%% The loops that call BIFs and functions in different ways.
calls() ->
    [{Loop, race(Loop, 2000)}
     || Loop <- [call_light, call_heavy, call_apply, call_fun, call_only, call_last,
                 call_apply_only, call_heavy_only, call_guard]].

%% The loops that call a BIF that traps to Erlang code. ERTS switches the
%% process at each trap, so each turn sends one tag.
traps() ->
    [{Loop, race(Loop, 30)} || Loop <- [trap_ext, trap_apply]].

%% The loop with erlang:bump_reductions/1, and the results and the errors
%% of erlang:bump_reductions/1.
bumps() ->
    [{bump_loop, race(bump_loop, 300)},
     {values, [erlang:bump_reductions(id(0)), erlang:bump_reductions(id(1 bsl 59 - 1))]},
     {errors, [bump_error(-1), bump_error(foo), bump_error(1 bsl 59),
               bump_error(-(1 bsl 59) - 1)]}].

bump_error(V) ->
    try erlang:bump_reductions(id(V))
    catch Class:Reason -> {Class, Reason}
    end.

%% The loops with erlang:yield/0 through call_ext, call_ext_only,
%% call_ext_last, and apply/3.
yields() ->
    [{Loop, race(Loop, 30)} || Loop <- [yield_op, yield_only, yield_last, yield_apply]].

%% The reductions that process_info(self(), reductions) shows for 100
%% iterations of each operation, and for a read of another process.
reductions() ->
    [{Op, measure(Op)} || Op <- ?OPS] ++ [{other, other()}].

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

%% The send instruction.
send_op(_, _, 0) -> ok;
send_op(P, T, N) ->
    P ! T,
    send_op(P, T, N - 1).

%% call_ext of erlang:send/2.
send_ext(_, _, 0) -> ok;
send_ext(P, T, N) ->
    erlang:send(P, T),
    send_ext(P, T, N - 1).

%% erlang:send/2 through apply/3 and its export.
send_apply(P, T, N) ->
    send_apply(id(erlang), id(send), [P, T], N).

send_apply(_, _, _, 0) -> ok;
send_apply(M, F, A, N) ->
    apply(M, F, A),
    send_apply(M, F, A, N - 1).

%% erlang:send/2 through an external fun.
send_fun(P, T, N) ->
    send_fun(P, T, id(fun erlang:send/2), N).

send_fun(_, _, _, 0) -> ok;
send_fun(P, T, F, N) ->
    F(P, T),
    send_fun(P, T, F, N - 1).

%% call_ext_only of erlang:send/2.
send_only(_, _, 0) -> ok;
send_only(P, T, N) ->
    only_send(P, T),
    send_only(P, T, N - 1).

only_send(P, T) -> erlang:send(P, T).

%% apply_last of erlang:send/2 through its export: the send can return
%% after a switch in a tail call.
send_apply_only(P, T, N) ->
    send_apply_only(id(erlang), id(send), [P, T], N).

send_apply_only(_, _, _, 0) -> ok;
send_apply_only(M, F, A, N) ->
    only_apply_send(M, F, A),
    send_apply_only(M, F, A, N - 1).

only_apply_send(M, F, A) -> apply(M, F, A).

%% A body call and its return, and call_ext of the light BIF
%% erlang:whereis/1.
call_light(_, _, 0) -> ok;
call_light(P, T, N) ->
    body(),
    erlang:whereis(nobody),
    P ! T,
    call_light(P, T, N - 1).

body() -> ok.

%% call_ext of the heavy BIF erlang:put/2. An immediate value for a key
%% that the dictionary has does not allocate (erl_process_dict.c:660-667).
call_heavy(_, _, 0) -> ok;
call_heavy(P, T, N) ->
    put(key, N),
    P ! T,
    call_heavy(P, T, N - 1).

%% erlang:whereis/1 through apply/3 and its export.
call_apply(P, T, N) ->
    call_apply(P, T, id(erlang), id(whereis), N).

call_apply(_, _, _, _, 0) -> ok;
call_apply(P, T, M, F, N) ->
    apply(M, F, [nobody]),
    P ! T,
    call_apply(P, T, M, F, N - 1).

%% erlang:whereis/1 through an external fun.
call_fun(P, T, N) ->
    call_fun(P, T, id(fun erlang:whereis/1), N).

call_fun(_, _, _, 0) -> ok;
call_fun(P, T, F, N) ->
    F(nobody),
    P ! T,
    call_fun(P, T, F, N - 1).

%% call_ext_only of the light BIF erlang:whereis/1.
call_only(_, _, 0) -> ok;
call_only(P, T, N) ->
    only_whereis(nobody),
    P ! T,
    call_only(P, T, N - 1).

only_whereis(Name) -> erlang:whereis(Name).

%% call_ext_last of the light BIF erlang:whereis/1.
call_last(_, _, 0) -> ok;
call_last(P, T, N) ->
    last_whereis(nobody),
    P ! T,
    call_last(P, T, N - 1).

last_whereis(Name) ->
    body(),
    erlang:whereis(Name).

%% apply_last of erlang:whereis/1 through its export.
call_apply_only(P, T, N) ->
    call_apply_only(P, T, id(erlang), id(whereis), N).

call_apply_only(_, _, _, _, 0) -> ok;
call_apply_only(P, T, M, F, N) ->
    only_apply(M, F),
    P ! T,
    call_apply_only(P, T, M, F, N - 1).

only_apply(M, F) -> apply(M, F, [nobody]).

%% call_ext_only of the heavy BIF erlang:put/2.
call_heavy_only(_, _, 0) -> ok;
call_heavy_only(P, T, N) ->
    only_put(N),
    P ! T,
    call_heavy_only(P, T, N - 1).

only_put(N) -> put(key, N).

%% The bif instruction of tuple_size/1.
call_guard(P, T, N) ->
    call_guard(P, T, id({T}), N).

call_guard(_, _, _, 0) -> ok;
call_guard(P, T, Tuple, N) ->
    P ! T,
    call_guard(P, T, Tuple, N - tuple_size(Tuple)).

%% call_ext of erlang:demonitor/2 with flush. The reference is not a
%% monitor, so the BIF traps to erts_internal:flush_monitor_messages/3
%% (bif.c:640-649). The loop makes the reference once, so that it does not
%% allocate.
trap_ext(P, T, N) ->
    trap_ext(P, T, make_ref(), N).

trap_ext(_, _, _, 0) -> ok;
trap_ext(P, T, Ref, N) ->
    P ! T,
    erlang:demonitor(Ref, [flush]),
    trap_ext(P, T, Ref, N - 1).

%% erlang:demonitor/2 with flush through apply/3 and its export.
trap_apply(P, T, N) ->
    trap_apply(P, T, id(erlang), id(demonitor), [make_ref(), [flush]], N).

trap_apply(_, _, _, _, _, 0) -> ok;
trap_apply(P, T, M, F, A, N) ->
    P ! T,
    apply(M, F, A),
    trap_apply(P, T, M, F, A, N - 1).

%% Entry 1, send 1 + 4, call_ext of erlang:bump_reductions/1 1 + 100.
bump_loop(_, _, 0) -> ok;
bump_loop(P, T, N) ->
    P ! T,
    erlang:bump_reductions(100),
    bump_loop(P, T, N - 1).

%% call_ext of erlang:yield/0.
yield_op(_, _, 0) -> ok;
yield_op(P, T, N) ->
    P ! T,
    true = erlang:yield(),
    yield_op(P, T, N - 1).

%% call_ext_only of erlang:yield/0.
yield_only(_, _, 0) -> ok;
yield_only(P, T, N) ->
    P ! T,
    true = only_yield(),
    yield_only(P, T, N - 1).

only_yield() -> erlang:yield().

%% call_ext_last of erlang:yield/0.
yield_last(_, _, 0) -> ok;
yield_last(P, T, N) ->
    P ! T,
    true = last_yield(),
    yield_last(P, T, N - 1).

last_yield() ->
    body(),
    erlang:yield().

%% erlang:yield/0 through apply/3 runs yield/0 of the erlang module.
yield_apply(P, T, N) ->
    yield_apply(P, T, id(erlang), N).

yield_apply(_, _, _, 0) -> ok;
yield_apply(P, T, M, N) ->
    P ! T,
    true = apply(M, yield, []),
    yield_apply(P, T, M, N - 1).

%% Run measured/3 in a new process, so that each window starts in a new
%% process.
measure(Op) ->
    spawn(?MODULE, measured, [self(), Op, 100]),
    receive {Op, Reds} -> Reds end.

%% Send {Op, Reds} to P, where Reds is the change of the reduction total
%% over N iterations of the operation Op.
measured(P, Op, N) ->
    X = argument(Op),
    Before = ?REDS,
    operation(Op, N, X),
    P ! {Op, ?REDS - Before}.

%% The argument of the operation. The sends go to a process that keeps
%% them, so the caller sends no signal to itself, and the collector gets
%% only the results.
argument(send) -> spawn(?MODULE, sink, []);
argument(yield_apply) -> id(erlang);
argument(apply) -> id(erlang);
argument('fun') -> id(fun erlang:whereis/1);
argument(guard) -> id({ok});
argument(_) -> none.

sink() ->
    receive stop -> ok end.

operation(_, 0, _) -> ok;
operation(Op, N, X) ->
    case Op of
        empty -> ok;
        body -> body();
        light -> erlang:whereis(nobody);
        heavy -> put(key, N);
        send -> X ! Op;
        bump -> erlang:bump_reductions(10);
        clamp -> erlang:bump_reductions(5000);
        yield -> erlang:yield();
        yield_apply -> apply(X, yield, []);
        apply -> apply(X, whereis, [nobody]);
        'fun' -> X(nobody);
        guard -> _ = tuple_size(X)
    end,
    operation(Op, N - 1, X).

%% The reduction total of another process, and the change of the total of
%% the caller over this read. The new process runs in the yield of the
%% caller and switches out in its own yield.
other() ->
    Pid = spawn(?MODULE, bump_yield, []),
    erlang:yield(),
    Before = ?REDS,
    {reductions, Reds} = process_info(Pid, reductions),
    [Reds, ?REDS - Before].

bump_yield() ->
    erlang:bump_reductions(100),
    erlang:yield().

%% The order of the 'DOWN' message of a process that returns from its
%% last frame and a message that another process sends after it. With
%% 3997, the return uses the last reduction of the budget: the process
%% switches out and exits in its next turn, so the message comes first.
%% With 3996, the process exits at once.
exits() ->
    [begin
         spawn(?MODULE, observe, [self(), N]),
         receive {order, Order} -> Order end
     end
     || N <- [3997, 3996]].

observe(Collector, N) ->
    {_, Ref} = spawn_monitor(?MODULE, burn_return, [N]),
    spawn(?MODULE, send_tag, [self()]),
    First = receive {'DOWN', Ref, _, _, normal} -> down; tag -> tag end,
    Second = receive {'DOWN', Ref, _, _, normal} -> down; tag -> tag end,
    Collector ! {order, [First, Second]}.

%% Entry 1, N + 1 entries of burn/1, the return of burn/1, and the
%% return of the last frame.
burn_return(N) ->
    burn(N),
    ok.

burn(0) -> ok;
burn(N) -> burn(N - 1).

send_tag(P) -> P ! tag.

%% ERTS only: the number of garbage collections in each loop. Each loop
%% must give 0, else a collection changes the reductions on ERTS.
gcs() ->
    [{Loop, gc_count(Loop)}
     || Loop <- [send_op, send_ext, send_apply, send_fun, send_only, send_apply_only,
                 call_light, call_heavy, call_apply, call_fun, call_only, call_last,
                 call_apply_only, call_heavy_only, call_guard, trap_ext, trap_apply, bump_loop,
                 yield_op, yield_only, yield_last, yield_apply]]
    ++ [{Op, gc_count(measured, Op)} || Op <- ?OPS].

gc_count(Loop) ->
    gc_count(Loop, a).

gc_count(Loop, Tag) ->
    Self = self(),
    {Pid, Ref} = spawn_monitor(?MODULE, traced, [Self, Loop, Tag]),
    erlang:trace(Pid, true, [garbage_collection, {tracer, Self}]),
    Pid ! go,
    gc_events(Ref, 0).

traced(Collector, Loop, Tag) ->
    receive go -> ok end,
    ?MODULE:Loop(Collector, Tag, 2000).

gc_events(Ref, Count) ->
    receive
        {'DOWN', Ref, _, _, normal} -> Count;
        {trace, _, gc_minor_start, _} -> gc_events(Ref, Count + 1);
        {trace, _, gc_major_start, _} -> gc_events(Ref, Count + 1);
        _ -> gc_events(Ref, Count)
    end.
