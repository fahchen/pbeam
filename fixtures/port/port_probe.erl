-module(port_probe).
-export([exit_status/0, output/0, command/0, close/0, owner_exit/0, epipe/0, errors/0,
         nul_name/0, sigchld_ignored/0, monitor/0, unsupported/1, terminal/0, high_fd/0, costs/1, measure/2]).

%% Each result replaces a port with the atom port, because the port numbers
%% of ERTS and PBeam differ.

sh() -> "/bin/sh".

%% The stty path of the Terra TTY backend: Terra.Terminal.TTY.run_stty/2
%% and close_port/1 (lib/terra/terminal/tty.ex:95-123), with an Elixir
%% string as the path and as each argument.
stty(Command) ->
    Port = open_port({spawn_executable, <<"/bin/sh">>},
                     [binary, exit_status, nouse_stdio, {args, [<<"-c">>, Command]}]),
    Result = receive
                 {Port, {exit_status, 0}} -> ok;
                 {Port, {exit_status, Status}} -> {error, {stty_exit, Status}}
             after 2000 -> {error, timeout}
             end,
    case erlang:port_info(Port) of
        undefined -> ok;
        _ -> port_close(Port)
    end,
    Result.

%% Turn ISIG off and on through stty, and read the mode after each step
%% through fd 4 of a nouse_stdio port. Stdin is the terminal of the run.
%% The results go out with io:put_chars/1, as in get_line_probe. When this
%% probe was added, io:format/2 with ~w stopped at the is_bitstr
%% instruction (opcode 129), which PBeam runs since #359.
terminal() ->
    Off = stty(<<"stty -isig 2>/dev/null">>),
    OffMode = isig(),
    On = stty(<<"stty isig 2>/dev/null">>),
    OnMode = isig(),
    io:put_chars([lists:join(" ", [show(R) || R <- [Off, OffMode, On, OnMode]]), "\n"]).

show({error, {stty_exit, Status}}) -> ["stty_exit ", integer_to_list(Status)];
show(Atom) -> atom_to_list(Atom).

isig() ->
    Script = "if stty -a 2>/dev/null | grep -q -- -isig; then printf off >&4; else printf on >&4; fi",
    Port = open_port({spawn_executable, sh()}, [binary, exit_status, nouse_stdio,
                                                {args, ["-c", Script]}]),
    {Mode, 0} = collect(Port, []),
    if
        Mode =:= <<"off">> -> off;
        Mode =:= <<"on">> -> on
    end.

%% The exit status of a program, and the port after it, with trap_exit.
exit_status() ->
    process_flag(trap_exit, true),
    Port = open_port({spawn_executable, sh()}, [binary, exit_status, nouse_stdio,
                                                {args, ["-c", "exit 3"]}]),
    Status = receive {Port, {exit_status, S}} -> S end,
    Info = erlang:port_info(Port),
    Exit = receive {'EXIT', Port, Reason} -> Reason end,
    Kill = open_port({spawn_executable, sh()}, [exit_status, binary,
                                                {args, ["-c", "kill -9 $$"]}]),
    Killed = receive {Kill, {exit_status, K}} -> K end,
    Atom = open_port({spawn_executable, <<"/bin/sh">>}, [binary, exit_status,
                                                         {args, [<<"-c">>, 'exit 4']}]),
    AtomStatus = receive {Atom, {exit_status, A}} -> A end,
    [kind(Port), kind(self()), Status, Info, Exit, catch_error(fun() -> port_close(Port) end),
     Killed, AtomStatus, flush()].

kind(Port) when is_port(Port) -> port;
kind(_) -> other.

%% The output of a program on fd 1 and fd 2, and on fd 4 with nouse_stdio.
output() ->
    process_flag(trap_exit, true),
    Stdio = open_port({spawn_executable, sh()},
                      [binary, exit_status, use_stdio, stderr_to_stdout,
                       {args, ["-c", "printf out; printf err >&2"]}]),
    Four = open_port({spawn_executable, sh()}, [binary, exit_status, nouse_stdio,
                                                {args, ["-c", "printf four >&4"]}]),
    Large = open_port({spawn_executable, sh()},
                      [binary, {args, ["-c", "head -c 100000 /dev/zero"]}]),
    [collect(Stdio, []), collect(Four, []), iolist_size(read_all(Large, [])), flush()].

%% The data of each message until the exit status, as one binary.
collect(Port, Data) ->
    receive
        {Port, {data, More}} -> collect(Port, [More | Data]);
        {Port, {exit_status, Status}} ->
            {unicode:characters_to_binary(lists:reverse(Data, []), unicode), Status}
    end.

read_all(Port, Data) ->
    receive
        {Port, {data, More}} -> read_all(Port, [More | Data]);
        {'EXIT', Port, normal} -> Data
    end.

%% Input of a program through port_command/2, and the counts of port_info/1.
%% The program echoes 5 bytes, and then waits for 1 more byte before it
%% exits. The probe reads all 5 bytes, and sends the last byte only after
%% the second port_info/1, so the port is open at each port_info/1 call and
%% the counts are 5. Else the program can exit and the port can close
%% before the second call, which then gives undefined.
command() ->
    process_flag(trap_exit, true),
    Self = self(),
    Port = open_port({spawn_executable, sh()},
                     [binary, {args, ["-c", "head -c 5; head -c 1 >/dev/null"]}]),
    Before = info(Port, Self),
    Sent = [port_command(Port, <<"hel">>), port_command(Port, ["l", $o])],
    Data = data(Port, 5, []),
    After = info(Port, Self),
    true = port_command(Port, <<"x">>),
    Exit = receive {'EXIT', Port, Reason} -> Reason end,
    [Before, Sent, Data, After, Exit, erlang:port_info(Port), flush()].

%% Size bytes of data from the port, as one binary. The program can write
%% its output in more than one part, and each part comes as one message.
data(_Port, 0, Data) ->
    iolist_to_binary(lists:reverse(Data, []));
data(Port, Size, Data) ->
    receive {Port, {data, D}} -> data(Port, Size - byte_size(D), [D | Data]) end.

info(Port, Self) ->
    Info = erlang:port_info(Port),
    {os_pid, OsPid} = lists:keyfind(os_pid, 1, Info),
    {id, Id} = lists:keyfind(id, 1, Info),
    [{K, V} || {K, V} <- Info, K =/= os_pid, K =/= id, V =/= [Self], V =/= Self]
        ++ [integer(OsPid), integer(Id), [K || {K, _} <- Info]].

integer(N) when is_integer(N) -> integer.

%% port_close/1, and the port BIFs after it.
close() ->
    process_flag(trap_exit, true),
    Port = open_port({spawn_executable, sh()}, [binary, {args, ["-c", "cat"]}]),
    Closed = port_close(Port),
    Exit = receive {'EXIT', Port, Reason} -> Reason end,
    [Closed, Exit, erlang:port_info(Port),
     catch_error(fun() -> port_close(Port) end),
     catch_error(fun() -> port_command(Port, <<"x">>) end), flush()].

%% The exit of the owner closes the port.
owner_exit() ->
    [owner_exit(normal), owner_exit(boom)].

owner_exit(Reason) ->
    Parent = self(),
    {Pid, Ref} = spawn_monitor(fun() ->
                                       Port = open_port({spawn_executable, sh()},
                                                        [binary, {args, ["-c", "cat"]}]),
                                       Parent ! {port, Port},
                                       receive stop -> exit(Reason) end
                               end),
    Port = receive {port, P} -> P end,
    Open = case erlang:port_info(Port) of [_ | _] -> open; undefined -> closed end,
    Pid ! stop,
    Down = receive {'DOWN', Ref, process, Pid, R} -> R end,
    receive after 100 -> ok end,
    [Open, Down, erlang:port_info(Port)].

%% A write to a program that closed its input closes the port with epipe:
%% the owner gets the EXIT message with trap_exit, and exits without it.
%% ERTS does not stop the program when the port closes
%% (sys_drivers.c:1039-1055), so the probe stops each program at the end.
%% Else the program keeps the stderr of the run until its sleep ends.
epipe() ->
    Parent = self(),
    {Pid, Ref} = spawn_monitor(fun() ->
                                       Port = closed_input(),
                                       Parent ! {program, os_pid(Port)},
                                       exit({result, write(Port)})
                               end),
    Down = receive {'DOWN', Ref, process, Pid, Reason} -> Reason end,
    First = receive {program, OsPid} -> OsPid end,
    process_flag(trap_exit, true),
    Port = closed_input(),
    Second = os_pid(Port),
    Result = [write(Port), Down],
    stop(First, Second),
    Result.

closed_input() ->
    open_port({spawn_executable, sh()},
              [binary, {args, ["-c", "exec 0<&-; printf ready; exec sleep 1"]}]).

os_pid(Port) ->
    {os_pid, OsPid} = lists:keyfind(os_pid, 1, erlang:port_info(Port)),
    OsPid.

write(Port) ->
    receive {Port, {data, <<"ready">>}} -> ok end,
    Sent = port_command(Port, <<"x">>),
    [Sent, flush(), erlang:port_info(Port)].

%% Send SIGTERM to the two programs, and wait until kill ends. The program
%% runs sleep with exec, so its OS pid is the pid of sleep.
stop(First, Second) ->
    Command = "kill " ++ integer_to_list(First) ++ " " ++ integer_to_list(Second),
    Kill = open_port({spawn_executable, sh()}, [binary, exit_status, {args, ["-c", Command]}]),
    receive {Kill, {exit_status, _}} -> ok end.

%% The open_port/2 errors and the errors of the port BIFs.
errors() ->
    Sh = {spawn_executable, sh()},
    Open = [catch_error(fun() -> open_port(Name, Settings) end)
            || {Name, Settings} <- [{{spawn_executable, "/nonexistent"}, [binary]},
                                    {{spawn_executable, "/etc/hosts"}, [binary]},
                                    {{spawn_executable, <<>>}, [binary]},
                                    {{spawn_executable, sh}, [binary]},
                                    {{spawn_executable, []}, [binary]},
                                    {{spawn_executable, 42}, [binary]},
                                    {{spawn_executable, sh(), x}, [binary]},
                                    {{other, sh()}, [binary]},
                                    {{}, [binary]},
                                    {foo, [binary]},
                                    {Sh, foo},
                                    {Sh, [bogus]},
                                    {Sh, [{bogus, 1}]},
                                    {Sh, [{a, b, c}]},
                                    {Sh, [{1, 2}]},
                                    {Sh, [binary | foo]},
                                    {Sh, [{args, [foo, 1]}]},
                                    {Sh, [{args, x}]},
                                    {Sh, [{args, ["a" | b]}]},
                                    {Sh, [nouse_stdio, stderr_to_stdout]},
                                    {foo, [bogus]}]],
    Internal = [erts_internal:open_port(Name, Settings)
                || {Name, Settings} <- [{{spawn_executable, "/nonexistent"}, [binary]},
                                        {{spawn_executable, []}, [binary]},
                                        {foo, [bogus]},
                                        {foo, [binary | foo]},
                                        {Sh, [nouse_stdio, stderr_to_stdout]},
                                        {Sh, [{args, [foo, 1]}]}]],
    Bifs = [catch_error(F)
            || F <- [fun() -> erlang:port_info(foo) end,
                     fun() -> erlang:port_info(42) end,
                     fun() -> port_close(foo) end,
                     fun() -> port_close(self()) end,
                     fun() -> port_command(foo, <<"x">>) end,
                     fun() -> port_command(self(), <<"x">>) end]],
    Port = open_port(Sh, [binary, {args, ["-c", "cat"]}]),
    Data = catch_error(fun() -> port_command(Port, foo) end),
    port_close(Port),
    [Open, Internal, Bifs, Data].

%% The port name of a binary path ends at its first byte 0.
nul_name() ->
    Port = open_port({spawn_executable, <<"/bin/sh", 0, "junk">>}, [binary, {args, ["-c", "cat"]}]),
    {name, Name} = lists:keyfind(name, 1, erlang:port_info(Port)),
    port_close(Port),
    Name.

%% The exit status of a program after os:set_signal(sigchld, ignore). ERTS
%% reaps its children in erl_child_setup, so the disposition of the BEAM
%% process does not change the status.
sigchld_ignored() ->
    ok = os:set_signal(sigchld, ignore),
    Port = open_port({spawn_executable, sh()}, [binary, exit_status, {args, ["-c", "exit 7"]}]),
    receive {Port, {exit_status, Status}} -> Status end.

%% The reductions of each port BIF in a new process, without the
%% reductions of the measure itself. Opts are the spawn options: the ERTS
%% reference run gives a large heap, so that no garbage collection occurs.
costs(Opts) ->
    Base = cost(none, Opts),
    [{Bif, cost(Bif, Opts) - Base}
     || Bif <- [open, command, info, info_closed, close, close_other]].

cost(Bif, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), Bif], Opts),
    receive {Pid, Cost} -> Cost end.

cost_arg(info_closed) ->
    Port = cat(),
    port_close(Port),
    Port;
cost_arg(close_other) ->
    Parent = self(),
    Pid = spawn(fun() -> Parent ! {self(), cat()}, receive stop -> ok end end),
    receive {Pid, Port} -> Port end;
cost_arg(Bif) when Bif =:= command; Bif =:= info; Bif =:= close -> cat();
cost_arg(_) -> none.

%% Make the argument in the measured process, so that the process owns
%% the port, and switch out, so that the BIF runs with the budget of a new
%% turn.
measure(Collector, Bif) ->
    Arg = cost_arg(Bif),
    erlang:yield(),
    {reductions, Before} = process_info(self(), reductions),
    cost_call(Bif, Arg),
    {reductions, After} = process_info(self(), reductions),
    Collector ! {self(), After - Before}.

cost_call(open, _) ->
    [erts_internal:open_port({spawn_executable, sh()}, [binary, {args, ["-c", "cat"]}])];
cost_call(command, Port) -> [erts_internal:port_command(Port, <<"x">>, [])];
cost_call(Bif, Port) when Bif =:= info; Bif =:= info_closed -> [erts_internal:port_info(Port)];
cost_call(Bif, Port) when Bif =:= close; Bif =:= close_other -> [erts_internal:port_close(Port)];
cost_call(none, _) -> [].

%% The output and the exit status of a port whose pipes get fds at or
%% above FD_SETSIZE.
high_fd() ->
    Port = open_port({spawn_executable, sh()}, [binary, exit_status,
                                                {args, ["-c", "printf hi; exit 5"]}]),
    collect(Port, []).

%% Each call stops the PBeam run with a runtime error.
unsupported(line) ->
    open_port({spawn_executable, sh()}, [binary, {line, 80}]);
unsupported(stream) ->
    open_port({spawn_executable, sh()}, [binary, stream]);
unsupported(list) ->
    open_port({spawn_executable, sh()}, []);
unsupported(spawn) ->
    open_port({spawn, "ls"}, [binary]);
unsupported(exec) ->
    open_port({spawn_executable, "/"}, [binary, exit_status]);
unsupported(force) ->
    port_command(cat(), <<"x">>, [force]);
unsupported(exit) ->
    exit(cat(), kill);
unsupported(register) ->
    register(port_probe_cat, cat());
unsupported(send) ->
    erlang:send(cat(), {self(), close});
unsupported(send_options) ->
    erlang:send(cat(), {self(), close}, []);
unsupported(bang) ->
    cat() ! {self(), close}.

%% A monitor of a port: the DOWN message at port_close/1, noproc for a
%% port that closed, the exit reason of the owner, and no DOWN message
%% after demonitor/1 or after the exit of the watcher.
monitor() ->
    process_flag(trap_exit, true),
    P = cat(),
    R = monitor(port, P),
    true = port_close(P),
    Closed = down(R, P),
    R2 = monitor(port, P),
    Noproc = down(R2, P),
    Q = cat(),
    R3 = monitor(port, Q),
    R4 = monitor(port, Q),
    true = demonitor(R4),
    {W, WRef} = spawn_monitor(fun() -> monitor(port, Q) end),
    receive {'DOWN', WRef, process, W, normal} -> ok end,
    Self = self(),
    Pid = spawn(fun() -> Port = cat(), Self ! {port, Port}, receive stop -> exit(boom) end end),
    O = receive {port, X} -> X end,
    R5 = monitor(port, O),
    Pid ! stop,
    Owner = down(R5, O),
    true = port_close(Q),
    Demonitored = down(R3, Q),
    [Closed, Noproc, Owner, Demonitored, flush()].

down(R, P) ->
    receive {'DOWN', R, port, P, Reason} -> Reason after 1000 -> missing end.

cat() ->
    open_port({spawn_executable, sh()}, [binary, {args, ["-c", "cat"]}]).

catch_error(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

%% The messages in the queue, with each port as the atom port.
flush() ->
    receive Message -> [normal(Message) | flush()] after 100 -> [] end.

normal(Port) when is_port(Port) -> port;
normal({A, B}) -> {normal(A), normal(B)};
normal({A, B, C}) -> {normal(A), normal(B), normal(C)};
normal(Term) -> Term.
