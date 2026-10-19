-module(ping_pong).
-export([ping/0, pong/0, reference/0, boom/0, wait/0, conflict_try/0, conflict_hold/0,
         monitor_watch/0, monitor_hold/0, unreg_hold/0, unreg_taker/0,
         info_watch/0, info_hold/0, info_item/0, info_all/0,
         down_wait/0, down_hold/0, atom_create/0, drain_wait/0, drain_hold/0,
         len_wait/0, len_hold/0, process_list/0, child_wait/0, child_send/0,
         child_hold/0, name_wait/0, name_send/0, reference_name/0,
         stamp_wait/0, stamp_send/0, term_wait/0, term_send/0, port_wait/0,
         kill_watch/0, kill_hold/0, kill_child/0, port_send/0, port_other/0,
         alive_wait/0, alive_send/0, alive_child/0, scheduler_count/0,
         wall_time/0, name_after_kill/0, name_kill_hold/0, name_kill_child/0,
         decode_atom/0, persistent_put/0, ets_new/0, port_transfer/0,
         port_transfer_hold/0, ports_list/0, port_use/0, port_give/0,
         port_term/0, port_term_give/0, atomics_new/0, timer_remote/0,
         timer_hold/0, alias_create/0, monitor_alias/0, alias_hold/0,
         timer_named/0, timer_read/0, timer_give/0, system_flag_depth/0,
         finish_loading/0, delete_module/0, regex_run/0, regex_give/0,
         group_leader_remote/0, leader_hold/0, pattern_match/0, pattern_give/0]).

-define(COUNT, 100).

%% The root of one scheduler. It waits for the other root, sends COUNT
%% messages in one burst, and returns whether the receiver saw them in order.
ping() ->
    true = register(ping, self()),
    Pong = wait_for(pong, 100000),
    % Cross the scheduler boundary with control signals before the messages.
    _ = link(Pong),
    true = unlink(Pong),
    _ = link(Pong),
    _ = monitor(process, Pong),
    _ = erlang:unique_integer(),
    _ = erlang:unique_integer([positive]),
    _ = erlang:unique_integer([monotonic]),
    _ = registered(),
    ok = send_pings(Pong, 1),
    Pong ! {done, self()},
    receive
        {done_ack, Next} -> {?COUNT, Next =:= ?COUNT + 1}
    after 60000 ->
        timeout
    end.

wait_for(_Name, 0) ->
    timeout;
wait_for(Name, Attempts) ->
    case whereis(Name) of
        undefined ->
            receive after 1 -> wait_for(Name, Attempts - 1) end;
        Pid ->
            Pid
    end.

send_pings(_Pid, N) when N > ?COUNT ->
    ok;
send_pings(Pid, N) ->
    Pid ! {ping, N},
    send_pings(Pid, N + 1).

%% The root of the other scheduler. It records the next expected sequence
%% number. A message out of order sets the state to -1, which the result
%% reports.
pong() ->
    true = register(pong, self()),
    pong_loop(1).

pong_loop(Next) ->
    receive
        {ping, N} when N =:= Next ->
            pong_loop(Next + 1);
        {ping, _N} ->
            pong_loop(-1);
        {done, From} ->
            From ! {done_ack, Next},
            ok
    after 60000 ->
        timeout
    end.

%% The reference entry for the pinned ERTS: one process for each root.
reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {ping, ping()} end),
    _ = spawn(fun() -> Self ! {pong, pong()} end),
    Ping = receive {ping, PingResult} -> PingResult after 60000 -> timeout end,
    Pong = receive {pong, PongResult} -> PongResult after 60000 -> timeout end,
    {Ping, Pong}.

%% A root that exits with an error.
boom() ->
    error(boom).

%% A root that waits for a message that no process can send.
wait() ->
    receive ok -> ok end.

%% The root that waits for the other root to hold the shared name, then finds
%% the name taken, stops the holder, and reports false.
conflict_try() ->
    ok = wait_registered(shared_name, 100000),
    false =
        try register(shared_name, self()) of
            _ -> unexpected
        catch
            error:badarg -> false
        end,
    case whereis(shared_name) of
        undefined -> ok;
        Pid -> Pid ! stop
    end,
    false.

%% The root that takes the shared name and keeps it until the other root
%% stops it.
conflict_hold() ->
    true = register(shared_name, self()),
    receive stop -> true after 60000 -> timeout end.

wait_registered(_Name, 0) ->
    timeout;
wait_registered(Name, Attempts) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_registered(Name, Attempts - 1) end;
        _Pid -> ok
    end.

%% The root that monitors the other root while it waits for the monitor to
%% leave the pending slot, then reports the reason of the DOWN message.
monitor_watch() ->
    Pid = wait_for(monitor_hold_name, 100000),
    Ref = monitor(process, Pid),
    receive
        {'DOWN', Ref, process, Pid, normal} -> true;
        {'DOWN', Ref, process, Pid, Other} -> Other
    after 60000 -> timeout
    end.

%% The root that exits normally after a short wait.
monitor_hold() ->
    true = register(monitor_hold_name, self()),
    receive after 10 -> ok end.

%% The root that holds a name until the other root unregisters it, then
%% registers the name again.
unreg_hold() ->
    true = register(unreg_name, self()),
    ok = wait_cleared(20000),
    true = register(unreg_name, self()),
    true.

wait_cleared(0) ->
    timeout;
wait_cleared(Attempts) ->
    case process_info(self(), registered_name) of
        [] ->
            ok;
        _ ->
            receive after 1 -> wait_cleared(Attempts - 1) end
    end.

%% The root that unregisters the name of the other root.
unreg_taker() ->
    ok = wait_registered(unreg_name, 20000),
    true = unregister(unreg_name),
    true.

%% The root that reads the registered name and the liveness of a process of
%% another scheduler, and then stops it.
info_watch() ->
    Pid = wait_for(info_name, 100000),
    {registered_name, info_name} = process_info(Pid, registered_name),
    true = is_process_alive(Pid),
    Pid ! stop,
    ok.

%% The root that holds the name of the other root.
info_hold() ->
    true = register(info_name, self()),
    receive stop -> ok after 60000 -> timeout end.

%% The root that asks for an item of a remote process that PBeam cannot read
%% yet. The run stops with the unsupported error.
info_item() ->
    Pid = wait_for(info_name, 100000),
    _ = process_info(Pid, message_queue_len),
    Pid ! stop,
    ok.

%% The root that reads the whole state of a remote process. PBeam cannot
%% read it yet, so the run stops with the unsupported error.
info_all() ->
    Pid = wait_for(info_name, 100000),
    _ = process_info(Pid),
    Pid ! stop,
    ok.

%% The root that waits without a timer for the DOWN of a remote process.
down_wait() ->
    Pid = wait_for(down_name, 100000),
    Ref = monitor(process, Pid),
    receive {'DOWN', Ref, process, Pid, normal} -> ok end.

%% The root that holds a name and exits after a short wait.
down_hold() ->
    true = register(down_name, self()),
    receive after 100 -> ok end.

%% The root that creates an atom at runtime. A run with scheduler threads
%% cannot create an atom yet.
atom_create() ->
    list_to_atom("new_scheduler_atom_" ++ integer_to_list(erlang:unique_integer([positive]))).

%% The root that busy-polls a shared name, then reads its mailbox without a
%% scheduling step.
drain_wait() ->
    true = register(drain_name, self()),
    Result = drain_poll(2000000),
    case whereis(drain_ready) of
        undefined -> ok;
        Sender -> Sender ! stop
    end,
    Result.

drain_poll(0) ->
    timeout;
drain_poll(Attempts) ->
    case whereis(drain_ready) of
        undefined -> drain_poll(Attempts - 1);
        _ -> receive Message -> Message after 0 -> timeout end
    end.

%% The root that sends a message, and then registers the name that the other
%% root polls.
drain_hold() ->
    Pid = wait_for(drain_name, 100000),
    Pid ! ready,
    true = register(drain_ready, self()),
    receive stop -> ok after 2000 -> timeout end.

%% The root that reads its own message queue length without a scheduling
%% step after the other root registers the name.
len_wait() ->
    true = register(len_name, self()),
    Result = len_poll(2000000),
    case whereis(len_ready) of
        undefined -> ok;
        Sender -> Sender ! stop
    end,
    Result.

len_poll(0) ->
    timeout;
len_poll(Attempts) ->
    case whereis(len_ready) of
        undefined -> len_poll(Attempts - 1);
        _ ->
            {message_queue_len, Length} = process_info(self(), message_queue_len),
            case Length of
                0 -> timeout;
                _ -> Length
            end
    end.

%% The root that sends a message, and then registers the name that the other
%% root polls.
len_hold() ->
    Pid = wait_for(len_name, 100000),
    Pid ! ready,
    true = register(len_ready, self()),
    receive stop -> ok after 2000 -> timeout end.

%% The root that lists the processes of the node.
process_list() ->
    processes().

%% The root that reads the queue length of its child without a scheduling
%% step after the other root registers the name.
child_wait() ->
    Pid = spawn(?MODULE, child_hold, []),
    Result = child_poll(Pid, 2000000),
    Pid ! stop,
    case whereis(child_ready) of
        undefined -> ok;
        Sender -> Sender ! stop
    end,
    Result.

child_poll(_Pid, 0) ->
    timeout;
child_poll(Pid, Attempts) ->
    case whereis(child_ready) of
        undefined -> child_poll(Pid, Attempts - 1);
        _ ->
            {message_queue_len, Length} = process_info(Pid, message_queue_len),
            case Length of
                0 -> timeout;
                _ -> Length
            end
    end.

%% The child that holds a name and waits for the stop message. Another
%% message that waits in its queue stays there.
child_hold() ->
    true = register(child_name, self()),
    receive stop -> ok after 2000 -> timeout end.

%% The root that sends a message to the child, and then registers the name
%% that the other root polls.
child_send() ->
    Pid = wait_for(child_name, 100000),
    Pid ! hold,
    true = register(child_ready, self()),
    receive stop -> ok after 2000 -> timeout end.

%% The root that receives a message through its registered name.
name_wait() ->
    true = register(name_send, self()),
    receive {hello, From} -> From ! {reply, ok} end,
    ok.

%% The root that sends to a registered name of another scheduler.
name_send() ->
    ok = wait_registered(name_send, 100000),
    name_send ! {hello, self()},
    receive {reply, Reply} -> Reply after 60000 -> timeout end.

%% The reference entry for the pinned ERTS: a send to a registered name.
reference_name() ->
    Self = self(),
    _ = spawn(fun() -> name_wait() end),
    ok = wait_registered(name_send, 100000),
    name_send ! {hello, Self},
    receive {reply, Reply} -> Reply after 60000 -> timeout end.

%% The root that compares its clock with the clock of the other scheduler.
stamp_wait() ->
    true = register(stamp_name, self()),
    receive {stamp, From, Offset, Time} ->
        Offset = erlang:time_offset(),
        true = erlang:monotonic_time() >= Time,
        From ! {stamp_ok, ok}
    end,
    ok.

%% The root that sends its clock values to the other scheduler.
stamp_send() ->
    ok = wait_registered(stamp_name, 100000),
    Offset = erlang:time_offset(),
    Time = erlang:monotonic_time(),
    stamp_name ! {stamp, self(), Offset, Time},
    receive {stamp_ok, Reply} -> Reply after 60000 -> timeout end.

%% The root that decodes a local-seeded term from the other scheduler.
term_wait() ->
    true = register(term_name, self()),
    receive {term, From, Binary} ->
        {value, 1} = binary_to_term(Binary),
        From ! {term_ok, ok}
    end,
    ok.

%% The root that sends a local-seeded term to the other scheduler.
term_send() ->
    ok = wait_registered(term_name, 100000),
    Binary = term_to_binary({value, 1}, [local]),
    term_name ! {term, self(), Binary},
    receive {term_ok, Reply} -> Reply after 60000 -> timeout end.

%% The root that waits without a receive timer for the output of a port.
port_wait() ->
    Port = open_port({spawn_executable, "/bin/echo"}, [binary, exit_status, {args, ["hi"]}]),
    receive
        {Port, {data, <<"hi\n">>}} -> ok;
        {Port, {exit_status, 0}} -> ok
    end.

%% The root that kills a process of the other scheduler and then checks
%% that it is not alive.
kill_watch() ->
    Child = wait_for(kill_name, 100000),
    exit(Child, kill),
    false = is_process_alive(Child),
    Holder = wait_for(kill_holder, 100000),
    Holder ! stop,
    ok.

%% The root that holds a process that the other root kills.
kill_hold() ->
    true = register(kill_holder, self()),
    _ = spawn(?MODULE, kill_child, []),
    receive stop -> ok after 60000 -> timeout end.

%% The process that holds a name and waits to be killed.
kill_child() ->
    true = register(kill_name, self()),
    receive never -> ok end.

%% The root that sends its port to the other scheduler.
port_send() ->
    Port = open_port({spawn_executable, "/usr/bin/true"}, [binary]),
    ok = wait_registered(port_name, 100000),
    port_name ! {port, self(), Port},
    receive {port_ok, Reply} -> Reply after 60000 -> timeout end.

%% The root that checks that its port differs from the port of the other
%% scheduler.
port_other() ->
    true = register(port_name, self()),
    Port = open_port({spawn_executable, "/usr/bin/true"}, [binary]),
    receive {port, From, Other} ->
        true = Other =/= Port,
        From ! {port_ok, ok}
    end,
    ok.

%% The root that checks the liveness of its child after another scheduler
%% killed it.
alive_wait() ->
    Pid = spawn(?MODULE, alive_child, []),
    Result = alive_poll(Pid, 2000000),
    case whereis(alive_ready) of
        undefined -> ok;
        Sender -> Sender ! stop
    end,
    Result.

alive_poll(_Pid, 0) ->
    timeout;
alive_poll(Pid, Attempts) ->
    case whereis(alive_ready) of
        undefined -> alive_poll(Pid, Attempts - 1);
        _ -> is_process_alive(Pid)
    end.

%% The child that holds a name and waits to be killed.
alive_child() ->
    true = register(alive_name, self()),
    receive never -> ok end.

%% The root that kills the child of the other scheduler, and then registers
%% the name that the other root polls.
alive_send() ->
    Pid = wait_for(alive_name, 100000),
    exit(Pid, kill),
    true = register(alive_ready, self()),
    receive stop -> ok after 6000 -> timeout end.

%% The root that reports the number of schedulers of the run.
scheduler_count() ->
    erlang:system_info(schedulers).

%% The root that disables the scheduler wall time. It waits for one reply
%% for each scheduler.
wall_time() ->
    erts_internal:scheduler_wall_time(false).

%% The root that kills the child of the other scheduler, and then reads its
%% registered name.
name_after_kill() ->
    Child = wait_for(name_kill, 100000),
    exit(Child, kill),
    undefined = process_info(Child, registered_name),
    Holder = wait_for(name_kill_holder, 100000),
    Holder ! stop,
    ok.

%% The root that holds the process that the other root kills.
name_kill_hold() ->
    true = register(name_kill_holder, self()),
    _ = spawn(?MODULE, name_kill_child, []),
    receive stop -> ok after 60000 -> timeout end.

%% The process that holds a name and waits to be killed.
name_kill_child() ->
    true = register(name_kill, self()),
    receive never -> ok end.

%% The root that decodes a term with a new atom. A run with scheduler
%% threads cannot create an atom.
decode_atom() ->
    binary_to_term(<<131, 119, 19, "review_runtime_atom">>).

%% The root that writes a persistent term. A run with scheduler threads
%% cannot share the table.
persistent_put() ->
    persistent_term:put(persistent_probe, 1).

%% The root that makes an ETS table. A run with scheduler threads cannot
%% share the tables.
ets_new() ->
    ets:new(ets_probe, []).

%% The root that connects its port to a process of the other scheduler.
port_transfer() ->
    Pid = wait_for(transfer_name, 100000),
    Port = open_port({spawn_executable, "/usr/bin/true"}, [binary]),
    true = port_connect(Port, Pid),
    ok.

%% The root that holds a name for the port transfer of the other root.
port_transfer_hold() ->
    true = register(transfer_name, self()),
    receive stop -> ok after 6000 -> timeout end.

%% The root that lists the ports of the node.
ports_list() ->
    erlang:ports().

%% The root that receives the port of the other scheduler and asks for its
%% information.
port_use() ->
    true = register(port_use_name, self()),
    receive {port, Port} ->
        erlang:port_info(Port)
    end.

%% The root that sends its port to the other scheduler.
port_give() ->
    %% The port stays open until the receiver uses it.
    Port = open_port({spawn_executable, "/bin/cat"}, [binary]),
    ok = wait_registered(port_use_name, 100000),
    port_use_name ! {port, Port},
    receive done -> ok after 6000 -> timeout end.

%% The root that uses the port of the other scheduler only as a term.
port_term() ->
    true = register(port_term_name, self()),
    receive {port, From, Port} ->
        List = erlang:port_to_list(Port),
        true = is_list(List),
        Binary = term_to_binary(Port),
        Port = binary_to_term(Binary),
        From ! done
    end,
    ok.

%% The root that sends its port to the other scheduler and waits for the
%% result.
port_term_give() ->
    Port = open_port({spawn_executable, "/usr/bin/true"}, [binary]),
    ok = wait_registered(port_term_name, 100000),
    port_term_name ! {port, self(), Port},
    receive done -> ok after 6000 -> timeout end.

%% The root that makes an atomic array. A run with scheduler threads
%% cannot share the arrays.
atomics_new() ->
    erts_internal:atomics_new(1, []).

%% The root that starts a timer for a process of the other scheduler.
timer_remote() ->
    Pid = wait_for(timer_name, 100000),
    _ = erlang:send_after(1000, Pid, tick),
    ok.

%% The root that holds a name for the timer of the other root.
timer_hold() ->
    true = register(timer_name, self()),
    receive stop -> ok after 6000 -> timeout end.

%% The root that makes an alias. A run with scheduler threads cannot share
%% the alias owners.
alias_create() ->
    erlang:alias().

%% The root that monitors a process of the other scheduler with an alias.
monitor_alias() ->
    Pid = wait_for(alias_name, 100000),
    erlang:monitor(process, Pid, [{alias, explicit_unalias}]).

%% The root that holds a name for the monitor of the other root.
alias_hold() ->
    true = register(alias_name, self()),
    receive stop -> ok after 6000 -> timeout end.

%% The root that starts a named timer. A run with scheduler threads cannot
%% route the message of a named timer.
timer_named() ->
    erlang:start_timer(1000, unregistered_timer_name, tick).

%% The root that reads a timer of the other scheduler.
timer_read() ->
    true = register(timer_read_name, self()),
    receive {timer, Reference} ->
        erlang:read_timer(Reference)
    end.

%% The root that sends its timer reference to the other scheduler.
timer_give() ->
    Reference = erlang:start_timer(60000, self(), tick),
    ok = wait_registered(timer_read_name, 100000),
    timer_read_name ! {timer, Reference},
    receive done -> ok after 6000 -> timeout end.

%% The root that sets the backtrace depth. The flag stays with each
%% scheduler in a run with scheduler threads.
system_flag_depth() ->
    erlang:system_flag(backtrace_depth, 0).

%% The root that loads code. The code table stays with each scheduler in a
%% run with scheduler threads.
finish_loading() ->
    erlang:finish_loading([]).

%% The root that deletes code. The code table stays with each scheduler in
%% a run with scheduler threads.
delete_module() ->
    erlang:delete_module(code_loading_probe).

%% The root that runs the compiled expression of the other scheduler.
regex_run() ->
    true = register(regex_name, self()),
    receive {regex, Pattern} ->
        re:run(<<"abc">>, Pattern, [])
    end.

%% The root that sends its compiled expression to the other scheduler.
regex_give() ->
    {ok, Pattern} = re:compile("abc", []),
    ok = wait_registered(regex_name, 100000),
    regex_name ! {regex, Pattern},
    receive done -> ok after 6000 -> timeout end.

%% The root that makes a process of the other scheduler its group leader.
group_leader_remote() ->
    Pid = wait_for(leader_name, 100000),
    true = group_leader(Pid, Pid),
    Pid ! stop,
    ok.

%% The root that holds a name for the group leader request.
leader_hold() ->
    true = register(leader_name, self()),
    receive stop -> ok after 6000 -> timeout end.

%% The root that matches with the compiled pattern of the other scheduler.
pattern_match() ->
    true = register(pattern_name, self()),
    receive {pattern, Pattern} ->
        binary:match(<<"abc">>, Pattern)
    end.

%% The root that sends its compiled pattern to the other scheduler.
pattern_give() ->
    Pattern = binary:compile_pattern(<<"abc">>),
    ok = wait_registered(pattern_name, 100000),
    pattern_name ! {pattern, Pattern},
    receive done -> ok after 6000 -> timeout end.
