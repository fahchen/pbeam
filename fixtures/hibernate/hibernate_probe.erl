-module(hibernate_probe).
-export([queued/0, later/0, apply/0, tail/0, bottom/0, order/0, send/2, id/1]).

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant module or function name.
id(X) -> X.

send(Pid, Msg) -> Pid ! Msg.

%% Return the messages in the queue, in queue order.
flush() ->
    receive M -> [M | flush()] after 0 -> [] end.

%% A message in the queue before the call: the call returns at once.
queued() ->
    Self = self(),
    Self ! early,
    R = erlang:hibernate(),
    {R, Self =:= self(), flush()}.

%% No message in the queue: the process waits for the message of a child.
later() ->
    Self = self(),
    spawn(?MODULE, send, [Self, late]),
    R = erlang:hibernate(),
    {R, Self =:= self(), flush()}.

%% The apply instruction and a fun call run the body in erlang.beam.
apply() ->
    Self = self(),
    M = ?MODULE:id(erlang),
    spawn(?MODULE, send, [Self, late]),
    A = M:hibernate(),
    Self ! queued,
    B = apply(M, ?MODULE:id(hibernate), []),
    spawn(?MODULE, send, [Self, fun_late]),
    F = ?MODULE:id(fun erlang:hibernate/0),
    C = F(),
    {A, B, C, flush()}.

%% call_ext_last: the call waits, and then returns from the frame.
tail() ->
    R = tail_last(),
    {R, flush()}.

tail_last() ->
    spawn(?MODULE, send, [self(), late]),
    erlang:hibernate().

%% A process whose first function is erlang:hibernate/0 waits, and then
%% returns from its last frame and exits with reason normal.
bottom() ->
    {Pid, Ref} = spawn_monitor(erlang, hibernate, []),
    A = receive {'DOWN', Ref, _, _, R1} -> {down, R1} after 50 -> waiting end,
    Pid ! wake,
    B = receive {'DOWN', Ref, _, _, R2} -> {down, R2} end,
    {A, B}.

%% With a message in the queue, the process still goes to the end of the
%% run queue, so the child runs before the call returns.
order() ->
    Self = self(),
    spawn(?MODULE, send, [Self, child]),
    Self ! own,
    R = erlang:hibernate(),
    {R, flush()}.
