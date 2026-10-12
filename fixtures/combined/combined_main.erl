%% The driver of the combined instruction pairs of
%% `docs/interpreter-dispatch-plan.md`, step 9. It calls each loop of
%% `combined_probe.erl` with the arguments of the test and collects the
%% results: the value of each clause, the exception class and reason, the
%% reduction charge of each pair, and the message order across the trap of
%% `length/1`.
%%
%% The loops are in another module, so that the type analysis of the
%% compiler cannot fold the clause test of a loop away.
%%
%% `tests/integration/combined_pairs.rs` runs `main/1` on PBeam and on the
%% pinned ERTS and compares the two results.
-module(combined_main).

-export([main/1, child/1]).

%% The argument is a list of 70000 elements, so that `length/1` of a pair
%% traps: the BIF runs for `16 *` the remaining reductions and then traps.
%% The list comes from the caller, because the two runs must use the same
%% list and a literal of that size is not a small term.
main(Long) ->
    {combined_probe:eq(0), combined_probe:eq(4),
     combined_probe:ne(3, 4), combined_probe:ne(3, 3),
     combined_probe:lt(-10), combined_probe:lt(0),
     combined_probe:ge(10), combined_probe:ge(0),
     caught(fun() -> combined_probe:raise_error(1) end),
     combined_probe:guard_fail(0), combined_probe:guard_fail(-5),
     combined_probe:guard_fail(foo),
     combined_probe:len_pair([a, b, c]), combined_probe:len_pair(Long),
     caught(fun() -> combined_probe:len_pair(1) end),
     combined_probe:len_call(2, [a, b, c], 0), combined_probe:len_call(1, Long, 0),
     caught(fun() -> combined_probe:bad_tail(1, foo) end),
     combined_probe:bad_tail(2, -3),
     reductions(Long), mailbox(Long), signal_order(Long)}.

%% The exception class and the reason of `Fun`, as a `catch` gives them.
caught(Fun) ->
    try Fun() of
        Value -> {ok, Value}
    catch
        Class:Reason -> {Class, Reason}
    end.

%% The reduction charge of each pair, as the change of the reduction total
%% of this process over the calls. `process_info/2` fetches each item with
%% no cost for the caller.
reductions(Long) ->
    {reductions, Before} = process_info(self(), reductions),
    combined_probe:eq(4),
    combined_probe:ne(3, 3),
    combined_probe:lt(0),
    combined_probe:ge(0),
    combined_probe:guard_fail(foo),
    combined_probe:len_pair(Long),
    combined_probe:len_call(1, [a, b, c], 0),
    combined_probe:bad_tail(2, -3),
    {reductions, After} = process_info(self(), reductions),
    After - Before.

%% The signal observation of the trap of `length/1`. The child process can
%% run only when the pair switches out, which the trap of `length/1` does.
%% Thus the message of the child is in the mailbox when the pair completes,
%% and `message_queue_len` is 1. A pair that does not trap leaves the queue
%% at 0.
signal_order(Long) ->
    Self = self(),
    spawn(?MODULE, child, [Self]),
    Length = combined_probe:len_call(1, Long, 0),
    {message_queue_len, Queued} = process_info(self(), message_queue_len),
    {Length, Queued}.

%% Send the tag of the child process of `signal_order/1`.
child(Self) ->
    Self ! child.

%% The message order across the trap of `length/1`. The two messages are in
%% the mailbox before the pair runs, so the pair must leave them in place
%% and the receive must give them back in order.
mailbox(Long) ->
    self() ! first,
    self() ! second,
    Len = combined_probe:len_call(1, Long, 0),
    receive
        M1 -> receive M2 -> {Len, M1, M2} end
    end.
