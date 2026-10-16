-module(hibernate3_probe).
-export([queued/0, later/0, dropped/0, undef/0, killed/0, errors/0, wide/0, applied/0, id/1, proc_lib/0, purger/0, woke/2, crash/0, wait_for_request/1]).

%% The function that a hibernated process calls when it wakes: it sends the
%% first message in its queue and the queue length to Parent.
woke(Parent, Tag) ->
    receive M ->
        {message_queue_len, N} = process_info(self(), message_queue_len),
        Parent ! {Tag, M, N}
    end.

%% A message in the queue before the call: the process does not wait.
queued() ->
    Self = self(),
    {_, Ref} = spawn_monitor(fun() ->
        self() ! early,
        erlang:hibernate(?MODULE, woke, [Self, queued])
    end),
    R = receive {queued, _, _} = Q -> Q end,
    {R, down(Ref)}.

%% No message in the queue: the process waits until a message arrives.
later() ->
    Self = self(),
    {Pid, Ref} = spawn_monitor(fun() -> erlang:hibernate(?MODULE, woke, [Self, later]) end),
    A = receive {later, _, _} = Early -> Early after 50 -> waiting end,
    B = process_info(Pid, message_queue_len),
    Pid ! wake,
    C = receive {later, _, _} = Q -> Q end,
    {A, B, C, down(Ref)}.

%% The call drops the stack and its catch: the exit after the wake-up ends
%% the process, and no message from the old frame follows.
dropped() ->
    Self = self(),
    {Pid, Ref} = spawn_monitor(fun() ->
        R = try erlang:hibernate(?MODULE, crash, []) catch C:E -> {C, E} end,
        Self ! {caught, R}
    end),
    Pid ! wake,
    D = down(Ref),
    Late = receive {caught, _} = C -> C after 50 -> none end,
    {D, Late}.

crash() ->
    receive _ -> exit(boom) end.

%% An undefined function fails after the wake-up, with no frame from before
%% the call in its stack trace.
undef() ->
    init_server(),
    {Pid, Ref} = spawn_monitor(fun() ->
        _ = [erlang:hibernate(?MODULE, missing, [1, 2])],
        ok
    end),
    Pid ! wake,
    down(Ref).

%% Exit signals and links reach a hibernated process.
killed() ->
    {Pid, Ref} = spawn_monitor(fun() -> erlang:hibernate(?MODULE, crash, []) end),
    A = receive {'DOWN', Ref, _, _, _} = Early -> Early after 50 -> waiting end,
    exit(Pid, stop),
    {A, down(Ref)}.

%% Argument errors come before the stack is dropped.
errors() ->
    [catch_error(fun() -> erlang:hibernate(M, F, A) end)
     || {M, F, A} <- [{1, f, []}, {m, "f", []}, {m, f, foo}, {m, f, [a | b]},
                      {m, f, lists:seq(1, 257)},
                      {m, f, lists:seq(1, 256) ++ foo},
                      {m, f, lists:seq(1, 255) ++ foo}]].

%% The argument check counts one cons cell more than MAX_ARG (255), so a
%% call with 256 arguments hibernates and fails in apply/3 after the wake-up.
wide() ->
    init_server(),
    {Pid, Ref} = spawn_monitor(fun() -> erlang:hibernate(?MODULE, missing, lists:seq(1, 256)) end),
    Pid ! wake,
    {undef, [{?MODULE, missing, A, []}]} = down(Ref),
    {undef, length(A)}.

%% The apply instruction and a fun call reach the BIF through its export.
applied() ->
    Self = self(),
    M = ?MODULE:id(erlang),
    F = ?MODULE:id(hibernate),
    {P1, R1} = spawn_monitor(fun() -> apply(M, F, [?MODULE, woke, [Self, apply]]) end),
    P1 ! wake,
    A = receive {apply, _, _} = Q1 -> Q1 end,
    Fun = ?MODULE:id(fun erlang:hibernate/3),
    {P2, R2} = spawn_monitor(fun() -> Fun(?MODULE, woke, [Self, 'fun']) end),
    P2 ! wake,
    B = receive {'fun', _, _} = Q2 -> Q2 end,
    {A, down(R1), B, down(R2)}.

id(X) -> X.

catch_error(F) ->
    try F() catch C:R -> {C, R} end.

%% proc_lib:hibernate/3 wakes in proc_lib:wake_up/3.
proc_lib() ->
    Self = self(),
    {Pid, Ref} = spawn_monitor(fun() -> proc_lib:hibernate(?MODULE, woke, [Self, proc_lib]) end),
    Pid ! wake,
    R = receive {proc_lib, _, _} = Q -> Q end,
    {R, down(Ref)}.

%% The idle path of erts_code_purger: a receive timeout hibernates into the
%% receive loop, which then serves later requests.
purger() ->
    Pid = spawn(?MODULE, wait_for_request, [0]),
    timer_wait(),
    A = request(Pid),
    timer_wait(),
    B = request(Pid),
    exit(Pid, kill),
    {A, B}.

wait_for_request(N) ->
    handle_request(
      receive Msg -> Msg
      after 10 -> erlang:hibernate(?MODULE, wait_for_request, [N + 1])
      end, N).

handle_request({request, From}, N) ->
    From ! {reply, N > 0},
    wait_for_request(0).

request(Pid) ->
    Pid ! {request, self()},
    receive {reply, _} = R -> R end.

timer_wait() ->
    receive after 50 -> ok end.

down(Ref) ->
    receive {'DOWN', Ref, process, _, R} -> R end.

%% Start the init process of `fixtures/undef/undef_init` when the run has
%% no boot, as `undef_probe:init_server/0` does. The error handler asks it
%% to load a module.
init_server() ->
    case erlang:function_exported(undef_init, start, 0) andalso whereis(init) =:= undefined of
        true -> undef_init:start();
        false -> ok
    end.
