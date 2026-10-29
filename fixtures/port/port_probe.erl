-module(port_probe).
-export([exit_status/0, output/0, command/0, close/0, owner_exit/0, epipe/0, errors/0,
         nul_name/0, spawn/0, options/0, directions/0, cd_status/0, sigchld_ignored/0, packet/0, packet_errors/0, eof/0, lines/0, monitor/0, fd/3, fd_steal/2, unsupported/1, terminal/0, high_fd/0, fd_closed/0, costs/1, measure/2, signals/0, busy/0, names/0]).

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
                                       %% With trap_exit the epipe exit signal
                                       %% of the port is a message, so the
                                       %% child sends its result before it
                                       %% exits. The parent then reads the
                                       %% messages in a fixed order.
                                       process_flag(trap_exit, true),
                                       Port = closed_input(),
                                       Parent ! {program, os_pid(Port)},
                                       Parent ! {result, write(Port)},
                                       exit(normal)
                               end),
    First = receive {program, OsPid} -> OsPid end,
    Written = receive {result, Result} -> Result end,
    Down = receive {'DOWN', Ref, process, Pid, Reason} -> Reason end,
    process_flag(trap_exit, true),
    Port = closed_input(),
    Second = os_pid(Port),
    Output = [Written, Down],
    stop(First, Second),
    Output.

closed_input() ->
    open_port({spawn_executable, sh()},
              [binary, {args, ["-c", "exec 0<&-; printf ready; exec sleep 1"]}]).

os_pid(Port) ->
    {os_pid, OsPid} = lists:keyfind(os_pid, 1, erlang:port_info(Port)),
    OsPid.

write(Port) ->
    receive {Port, {data, <<"ready">>}} -> ok end,
    Sent = port_command(Port, <<"x">>),
    Exit = receive {'EXIT', Port, Reason} -> [{'EXIT', port, Reason}] after 5000 -> [] end,
    [Sent, Exit, erlang:port_info(Port)].

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

%% {packet, N} in both directions: cat echoes each packet with its header,
%% and the port gives each packet as one message, also when a read holds
%% more than one packet or a part of a packet. A packet that does not fit
%% in its header closes the port with einval, and the end of file drops a
%% part of a packet.
packet() ->
    process_flag(trap_exit, true),
    Four = open_port({spawn_executable, sh()}, [{packet, 4}, binary, {args, ["-c", "cat"]}]),
    Sent = [port_command(Four, <<"hello">>), port_command(Four, [<<>>]),
            port_command(Four, ["ab", $c])],
    Echo = [receive {Four, {data, D}} -> D end || _ <- [1, 2, 3]],
    Counts = [erlang:port_info(Four, input), erlang:port_info(Four, output)],
    true = port_close(Four),
    One = open_port({spawn_executable, sh()},
                    [binary, {packet, 1}, exit_status,
                     {args, ["-c", "printf '\\002h'; sleep 0.1; printf 'i\\000\\001'; sleep 0.1; printf x"]}]),
    Two = open_port({spawn_executable, sh()},
                    [{packet, 2}, binary, exit_status,
                     {args, ["-c", "printf '\\000'; sleep 0.1; printf '\\003abc\\000\\005ab'"]}]),
    Split = [collect_packets(One, []), collect_packets(Two, [])],
    Limits = [too_large(1, 255), too_large(1, 256), too_large(2, 65535), too_large(2, 65536)],
    [Sent, Echo, Counts, Split, Limits, flush()].

collect_packets(Port, Packets) ->
    receive
        {Port, {data, Packet}} -> collect_packets(Port, [Packet | Packets]);
        {Port, {exit_status, Status}} -> {lists:reverse(Packets, []), Status}
    end.

%% The result of a packet of Size bytes on a port with a header of Header
%% bytes: the size of the echo, or the exit reason of the port.
too_large(Header, Size) ->
    Port = open_port({spawn_executable, sh()}, [{packet, Header}, binary, {args, ["-c", "cat"]}]),
    true = port_command(Port, binary:copy(<<"x">>, Size)),
    receive
        {Port, {data, Echo}} -> port_close(Port), byte_size(Echo);
        {'EXIT', Port, Reason} -> Reason
    end.

%% The settings of {packet, N} that ERTS does not accept, and the small
%% integers whose low 32 bits ERTS takes as the header size: each of them
%% echoes a packet through cat.
packet_errors() ->
    Sh = {spawn_executable, sh()},
    [erts_internal:open_port(Sh, [binary, {packet, N}])
     || N <- [0, 3, 8, -1, x, 1 bsl 64, (1 bsl 32) + 3, (1 bsl 59) + 4]]
        ++ [catch_error(fun() -> open_port(Sh, [binary, {packet, 3}]) end)]
        ++ [echo(N) || N <- [(1 bsl 32) + 1, (1 bsl 32) + 2, (1 bsl 32) + 4, 1 - (1 bsl 32)]].

echo(Header) ->
    Port = open_port({spawn_executable, sh()}, [binary, {packet, Header}, {args, ["-c", "cat"]}]),
    true = port_command(Port, <<"hi">>),
    receive {Port, {data, Data}} -> port_close(Port), Data end.

%% The eof setting: the end of file sends {Port, eof} after the exit status,
%% and the port stays open until port_close/1 or a failed write. The
%% overlapped_io setting changes nothing on Unix, as in the settings of
%% inet_gethost_native.
eof() ->
    process_flag(trap_exit, true),
    Status = open_port({spawn_executable, sh()},
                       [binary, eof, exit_status, {args, ["-c", "printf hi; exit 3"]}]),
    Messages = [receive {Status, M} -> M end || _ <- [1, 2, 3]],
    Open = [erlang:port_info(Status, input), erlang:port_info(Status, connected) =:= {connected, self()}],
    Write = port_command(Status, <<"x">>),
    Exit = receive {'EXIT', Status, R} -> R end,
    Plain = open_port({spawn_executable, sh()}, [eof, binary, {args, ["-c", "printf hi"]}]),
    PlainMessages = [receive {Plain, M} -> M end || _ <- [1, 2]],
    Quiet = receive {Plain, Other} -> Other after 100 -> none end,
    Close = port_close(Plain),
    PlainExit = receive {'EXIT', Plain, PR} -> PR end,
    Gethost = open_port({spawn_executable, sh()},
                        [overlapped_io, {args, ["-c", "printf '\\000\\000\\000\\002hi\\000'"]},
                         {packet, 4}, eof, binary]),
    GethostMessages = [receive {Gethost, M} -> M end || _ <- [1, 2]],
    true = port_close(Gethost),
    [normal(Messages), Open, Write, Exit, PlainMessages, Quiet, Close, PlainExit,
     GethostMessages, flush()].

%% The line and list data modes: {eol, Line} and {noeol, Part} for a line
%% longer than L and for the last line at the end of file, \r\n as a newline,
%% a list of bytes without binary, the low 32 bits of L, the bad values
%% of L, and line with packet.
lines() ->
    process_flag(trap_exit, true),
    Run = fun(Command, Settings) ->
                  Port = open_port({spawn, Command}, [exit_status | Settings]),
                  port_messages(Port)
          end,
    Cases = [Run("printf 'abcdefgh\\nxy\\r\\nab\\rc\\r\\r\\nlast'", [{line, 4}, binary]),
             Run("printf 'abcd\\nabcd\\r\\nabcde\\r\\nab'", [{line, 4}]),
             Run("printf 'abcd\\rxy'", [{line, 4}, binary]),
             Run("printf hi", []),
             Run("printf hi", [{packet, 2}, stream, line]),
             Run("printf 'abcd\\n'", [{line, 1 bsl 32 + 3}])],
    Eof = open_port({spawn, "printf 'abcd\\r'"}, [{line, 4}, eof, binary]),
    EofMessages = [receive {Eof, M} -> M end || _ <- [1, 2, 3]],
    true = port_close(Eof),
    Errors = [catch_error(fun() -> open_port({spawn, "true"}, Settings) end)
              || Settings <- [[{line, 0}], [{line, -1}], [{line, 1 bsl 60}], [{line, a}],
                              [{packet, 1}, line], [{line, 1 bsl 32}, {packet, 2}]]],
    [Cases, EofMessages, Errors, flush()].

port_messages(Port) ->
    receive
        {Port, M} -> [M | port_messages(Port)];
        {'EXIT', Port, R} -> [R]
    end.

%% The spawn port kind: a command runs as `sh -c "exec Command"`, and the
%% port name is the command, and $0 is sh. {args, Args} gives badarg. A command that
%% the shell does not find exits with 127. A first word that names a
%% driver opens the driver, and exit_status with a driver gives badarg.
spawn() ->
    process_flag(trap_exit, true),
    Echo = open_port({spawn, "echo hi; exit 3"}, [binary, exit_status]),
    Name = erlang:port_info(Echo, name),
    EchoMessages = [receive {Echo, M} -> M end || _ <- [1, 2]],
    Args = open_port({spawn, <<"echo $0 $#">>}, [binary, exit_status]),
    ArgsMessages = [receive {Args, M} -> M end || _ <- [1, 2]],
    Cat = open_port({spawn, cat}, [binary]),
    true = port_command(Cat, <<"x">>),
    CatData = receive {Cat, C} -> C end,
    true = port_close(Cat),
    Missing = open_port({spawn, "pbeam_no_such_command"}, [binary, exit_status]),
    MissingStatus = receive {Missing, {exit_status, S}} -> S end,
    Tcp = open_port({spawn, "tcp_inet"}, [binary]),
    TcpName = erlang:port_info(Tcp, name),
    true = port_close(Tcp),
    Udp = open_port({spawn, "udp_inet x"}, [binary]),
    UdpName = erlang:port_info(Udp, name),
    true = port_close(Udp),
    Errors = [catch_error(fun() -> open_port(Kind, [binary, exit_status]) end)
              || Kind <- [{spawn, "tcp_inet"}, {spawn, []}, {spawn, 42}, {spawn, "a", b}]]
        ++ [catch_error(fun() -> open_port(Kind, [binary, {args, Vector}]) end)
            || Kind <- [{spawn, "true"}, {spawn, "tcp_inet"}, {spawn_driver, "tcp_inet"}], Vector <- [[], ["a"]]],
    [Name, EchoMessages, ArgsMessages, CatData, MissingStatus, TcpName, UdpName, Errors, flush()].

%% The env, arg0, cd, hide, and parallelism options: the argv[0], the
%% environment, and the directory that the program sees, the parallelism
%% item of port_info/2, the directory of the node for {cd, []}, and the
%% badarg of each bad option.
options() ->
    process_flag(trap_exit, true),
    os:putenv("PBEAM_PROBE_PUT", "p"),
    os:putenv("PBEAM_PROBE_KEEP", "k"),
    Show = "printf '%s %s %s %s %s %s %s' $0 ${PBEAM_A-unset} ${PBEAM_B-unset} "
        "${PBEAM_PROBE_PUT-unset} ${PBEAM_PROBE_KEEP-unset} ${PBEAM_GONE-unset} $(pwd)",
    Exe = open_port({spawn_executable, sh()},
                    [binary, exit_status, {arg0, "zz"}, {args, ["-c", Show]},
                     {env, [{"PBEAM_GONE", "x"}]},
                     {env, [{"PBEAM_A", "1"}, {"PBEAM_PROBE_PUT", false}, {"PBEAM_B", []},
                            {["PBEAM_", "C"], "c"}]},
                     {cd, "/"}, hide, {parallelism, true}]),
    Parallel = erlang:port_info(Exe, parallelism),
    ExeMessages = [receive {Exe, M} -> M end || _ <- [1, 2]],
    Shell = open_port({spawn, "sh -c 'pwd; echo $PBEAM_C'"},
                      [binary, exit_status, {env, [{"PBEAM_C", "c"}]}, {cd, <<"/">>},
                       {parallelism, false}]),
    ShellParallel = erlang:port_info(Shell, parallelism),
    ShellMessages = collect_status(Shell, <<>>),
    Cat = cat(),
    Default = erlang:port_info(Cat, parallelism),
    true = port_close(Cat),
    NoCd = open_port({spawn, "pwd"}, [binary, exit_status, {cd, []}]),
    NoCdMessages = collect_status(NoCd, <<>>),
    Here = open_port({spawn, "pwd"}, [binary, exit_status]),
    HereMessages = collect_status(Here, <<>>),
    Errors = [catch_error(fun() -> open_port(Name, [binary | Opts]) end)
              || {Name, Opts} <- [{{spawn, "true"}, [{env, [{"A=B", "1"}]}]},
                                  {{spawn, "true"}, [{env, [{"A", 1}]}]},
                                  {{spawn, "true"}, [{env, [{"A"}]}]},
                                  {{spawn, "true"}, [{env, x}]},
                                  {{spawn, "true"}, [{cd, 1}]},
                                  {{spawn, "true"}, [{parallelism, x}]},
                                  {{spawn, "true"}, [{arg0, "x"}]},
                                  {{spawn_executable, sh()}, [{arg0, 1}]}]],
    [Parallel, ExeMessages, ShellParallel, ShellMessages, Default, NoCdMessages =:= HereMessages, Errors,
     flush()].

collect_status(Port, Data) ->
    receive
        {Port, {data, More}} -> collect_status(Port, <<Data/binary, More/binary>>);
        {Port, {exit_status, S}} -> [Data, S]
    end.

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
     || Bif <- [open, command, info, info_closed, close, close_other, exit, message_close, message_connect]].

cost(Bif, Opts) ->
    Pid = spawn_opt(?MODULE, measure, [self(), Bif], Opts),
    receive {Pid, Cost} -> Cost end.

cost_arg(info_closed) ->
    Port = cat(),
    port_close(Port),
    Port;
cost_arg(Bif) when Bif =:= close_other; Bif =:= exit ->
    Parent = self(),
    Pid = spawn(fun() -> Parent ! {self(), cat()}, receive stop -> ok end end),
    receive {Pid, Port} -> Port end;
cost_arg(Bif) when Bif =:= command; Bif =:= info; Bif =:= close;
                   Bif =:= message_close; Bif =:= message_connect -> cat();
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
cost_call(exit, Port) -> [exit(Port, shutdown)];
cost_call(message_close, Port) -> [Port ! {self(), close}];
cost_call(message_connect, Port) -> [Port ! {self(), {connect, self()}}];
cost_call(none, _) -> [].

%% The output and the exit status of a port whose pipes get fds at or
%% above FD_SETSIZE.
high_fd() ->
    Port = open_port({spawn_executable, sh()}, [binary, exit_status,
                                                {args, ["-c", "printf hi; exit 5"]}]),
    collect(Port, []).

%% A port on fd 200, which is not open, opens; its write fails with ebadf.
%% An fd at or above the soft RLIMIT_NOFILE gives einval.
fd_closed() ->
    process_flag(trap_exit, true),
    Port = open_port({fd, 200, 200}, [binary]),
    Idle = flush(),
    Sent = port_command(Port, <<"x">>),
    Exit = receive {'EXIT', Port, Reason} -> [{'EXIT', port, Reason}] after 5000 -> [] end,
    High = catch_error(fun() -> open_port({fd, 2000000, 2000000}, [binary]) end),
    %% The in option keeps no write side, so a write fails with ebadf; the
    %% out option keeps no read side (sys_drivers.c:385-401).
    In = open_port({fd, 200, 200}, [binary, in]),
    InSent = port_command(In, <<"x">>),
    InExit = receive {'EXIT', In, InReason} -> InReason after 5000 -> none end,
    Out = open_port({fd, 200, 200}, [binary, out]),
    OutIdle = flush(),
    OutClosed = port_close(Out),
    [Idle, Sent, Exit, erlang:port_info(Port), High, [InSent, InExit, OutIdle, OutClosed]].

%% The in and out options of open_port/2: in keeps the read side only and
%% out the write side only (erl_bif_port.c:991-994, 1015-1016). The child
%% setup program moves the output pipe to the child only with in and the
%% input pipe only with out (erl_child_setup.c:315-338).
directions() ->
    process_flag(trap_exit, true),
    %% in: the port sends the output of the program and its exit status.
    In = open_port({spawn_executable, sh()},
                   [in, binary, exit_status, {args, ["-c", "printf hi"]}]),
    InData = receive {In, {data, Data}} -> Data after 2000 -> timeout end,
    InStatus = receive {In, {exit_status, Status}} -> Status after 2000 -> timeout end,
    %% in: a write to the port fails with ebadf, because the driver keeps
    %% no write end (sys_drivers.c:1320-1329, 1145, 1177).
    InWrite = open_port({spawn_executable, sh()},
                        [in, binary, {args, ["-c", "sleep 1"]}]),
    receive after 100 -> ok end,
    InSent = port_command(InWrite, <<"x">>),
    InExit = receive {'EXIT', InWrite, InReason} -> InReason after 2000 -> timeout end,
    %% out: the port writes the input of the program and reads nothing;
    %% the exit status of the child never arrives
    %% (sys_drivers.c:746-753 with no ifd).
    Out = open_port({spawn_executable, sh()},
                    [out, binary, exit_status,
                     {args, ["-c", "read Line; test \"x$Line\" = xhello"]}]),
    OutSent = port_command(Out, <<"hello\n">>),
    OutRead = receive {Out, Message} -> Message after 500 -> none end,
    %% out: a write after the child exited fails with epipe
    %% (sys_drivers.c:1180-1184).
    receive after 500 -> ok end,
    OutAgain = port_command(Out, <<"x">>),
    OutExit = receive {'EXIT', Out, OutReason} -> OutReason after 2000 -> timeout end,
    [InData, InStatus, InSent, InExit, OutSent, OutRead, OutAgain, OutExit, flush()].

%% The exit status of a start that ran no program: the child setup
%% program exits with the errno of a chdir it cannot do and of an execve
%% that fails (erl_child_setup.c:302-306, 361-369).
cd_status() ->
    process_flag(trap_exit, true),
    Missing = open_port({spawn_executable, sh()},
                        [binary, exit_status, nouse_stdio, {args, ["-c", "printf never"]},
                         {cd, "/pbeam_no_such_dir"}]),
    MissingStatus = receive {Missing, {exit_status, NoDir}} -> NoDir after 2000 -> timeout end,
    MissingExit = receive {'EXIT', Missing, MissingReason} -> MissingReason after 2000 -> timeout end,
    File = open_port({spawn_executable, sh()}, [binary, exit_status, {cd, "/etc/hosts"}]),
    FileStatus = receive {File, {exit_status, NotDir}} -> NotDir after 2000 -> timeout end,
    %% The access check passes for a directory; the execve of the child
    %% fails with eacces (sys_drivers.c:518-527, erl_child_setup.c:361-369).
    Dir = open_port({spawn_executable, "/"}, [binary, exit_status]),
    DirStatus = receive {Dir, {exit_status, Exec}} -> Exec after 2000 -> timeout end,
    [MissingStatus, MissingExit, FileStatus, DirStatus, flush()].

%% Each call stops the PBeam run with a runtime error.
unsupported(line) ->
    open_port({spawn_executable, sh()}, [binary, {line, 1 bsl 31}]);
unsupported(empty_line) ->
    open_port({spawn_executable, sh()}, [binary, {line, 1 bsl 32}]);
unsupported(tcp_options) ->
    erlang:port_command(open_port({spawn_driver, "tcp_inet"}, [binary]), <<"x">>, [nosuspend]);
unsupported(fd_list) ->
    open_port({fd, 200, 200}, []).

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

%% The queue of a spawn port and its busy state (sys_drivers.c:1058-1135,
%% 1455-1493): a program that reads nothing takes 64 KiB in its pipe, the
%% queue makes the port busy at the high limit, nosuspend gives false, and
%% a busy port suspends the sender while other processes run.
busy() ->
    process_flag(trap_exit, true),
    B = <<0:8192>>,
    P = open_port({spawn, "sleep 5"}, [binary]),
    Full = fill(P, B, 0, 2000),
    %% The pipe size depends on the kernel, so the probe checks that the
    %% queue reached the high limit and holds less than one more write.
    Queue = queue_between(P, 8192, 8192 + 1024),
    Force = error_of(fun() -> erlang:port_command(P, B, [force]) end),
    BadOpt = error_of(fun() -> erlang:port_command(P, B, [foo]) end),
    Low = open_port({spawn, "sleep 5"}, [binary, {busy_limits_port, {100, 200}}]),
    LowFull = fill(Low, B, 0, 2000),
    LowQueue = queue_between(Low, 200, 200 + 1024),
    Off = open_port({spawn, "sleep 5"}, [binary, {busy_limits_port, disabled}]),
    OffFull = fill(Off, B, 0, 100),
    Swapped = is_port(open_port({spawn, "true"}, [{busy_limits_port, {300, 200}}, {busy_limits_msgq, {1 bsl 63, 1 bsl 63}}])),
    Bad = [error_of(fun() -> open_port({spawn, "true"}, [{busy_limits_port, L}]) end)
           || L <- [{0, 10}, {10, 1 bsl 64}, foo, {1, 2, 3}]],
    BadMsgq = error_of(fun() -> open_port({spawn, "true"}, [{busy_limits_msgq, {0, 1}}]) end),
    [exit(Q, kill) || Q <- [P, Low, Off]],
    Slow = open_port({spawn_executable, sh()}, [binary, {args, ["-c", "sleep 1; cat >/dev/null"]}]),
    Self = self(),
    Ticker = spawn(fun() -> tick(Self, 0) end),
    Sent = lists_all([port_command(Slow, B) || _ <- lists_seq(100)]),
    Ticker ! stop,
    Ticked = receive {ticks, N} -> N > 0 after 2000 -> missing end,
    Closing = open_port({spawn_executable, sh()},
                        [binary, {busy_limits_port, disabled}, {args, ["-c", "sleep 1; cat >/dev/null"]}]),
    Queued = fill(Closing, B, 0, 70),
    Closed = port_close(Closing),
    Exit = receive {'EXIT', Closing, Reason} -> Reason after 3000 -> missing end,
    %% A queue whose reader exits fails the port with epipe.
    Gone = open_port({spawn_executable, sh()}, [binary, out, {busy_limits_port, disabled}, {args, ["-c", "sleep 0.3"]}]),
    GoneQueued = fill(Gone, B, 0, 70),
    GoneExit = receive {'EXIT', Gone, GoneReason} -> GoneReason after 3000 -> missing end,
    [element(2, Full), Queue, Force, BadOpt, element(2, LowFull), LowQueue, OffFull, Swapped, Bad, BadMsgq, Sent, Ticked,
     Queued, Closed, Exit, GoneQueued, GoneExit].

queue_between(P, Min, Max) ->
    {queue_size, Size} = erlang:port_info(P, queue_size),
    Size >= Min andalso Size < Max.

fill(P, B, N, Max) ->
    case erlang:port_command(P, B, [nosuspend]) of
        true when N + 1 < Max -> fill(P, B, N + 1, Max);
        Result -> {N + 1, Result}
    end.

error_of(F) ->
    try F() of _ -> ok catch Class:Reason -> {Class, Reason} end.

tick(Parent, N) ->
    receive stop -> Parent ! {ticks, N} after 10 -> tick(Parent, N + 1) end.

lists_seq(0) -> [];
lists_seq(N) -> [N | lists_seq(N - 1)].

lists_all([true | T]) -> lists_all(T);
lists_all([]) -> true;
lists_all(_) -> false.

%% A registered port (register.c:175-253): its name in whereis/1,
%% registered/0, port_info/1,2, each send form, and the port BIFs, and the
%% loss of the name at unregister/1 and at the close (io.c:3904-3905).
names() ->
    process_flag(trap_exit, true),
    E = fun(F) -> try F() catch C:R:S -> {C, R, error_cause(S)} end end,
    P = cat(),
    Q = cat(),
    Before = [erlang:port_info(P, registered_name), lists:keyfind(registered_name, 1, erlang:port_info(P))],
    Reg = register(pp, P),
    Named = [whereis(pp) =:= P, lists:member(pp, registered()), erlang:port_info(P, registered_name),
             hd(erlang:port_info(P)), E(fun() -> register(pp2, P) end), E(fun() -> register(pp, Q) end),
             E(fun() -> register(pp, self()) end)],
    pp ! {self(), {command, <<"a">>}},
    A = data(P),
    erlang:send(pp, {self(), {command, <<"b">>}}),
    B = data(P),
    {pp, node()} ! {self(), {command, <<"c">>}},
    C = data(P),
    true = port_command(pp, <<"d">>),
    D = data(P),
    Info = [erlang:port_info(pp, connected) =:= {connected, self()}, erlang:port_info(pp, registered_name)],
    R = monitor(port, pp),
    U1 = unregister(pp),
    U2 = [whereis(pp), erlang:port_info(P, registered_name), E(fun() -> port_command(pp, <<"e">>) end)],
    Unreg = [U1, U2, register(pp, P)],
    true = port_close(pp),
    Down = receive {'DOWN', R, port, Item, Reason} -> {Item, Reason} after 1000 -> missing end,
    Closed = [whereis(pp), lists:member(pp, registered()), E(fun() -> register(pp, P) end),
              E(fun() -> port_close(pp) end)],
    true = register(qq, Q),
    exit(Q, kill),
    Killed = receive {'EXIT', Q, KillReason} -> KillReason after 1000 -> missing end,
    [Before, Reg, Named, [A, B, C, D], Info, Unreg, Down, Closed, Killed, whereis(qq), flush()].

error_cause([{_, _, _, Info} | _]) ->
    case lists:keyfind(error_info, 1, Info) of
        {error_info, #{cause := Cause}} -> Cause;
        _ -> none
    end.

data(P) ->
    receive {P, {data, Data}} -> Data after 1000 -> missing end.

down(R, P) ->
    receive {'DOWN', R, port, P, Reason} -> Reason after 1000 -> missing end.

cat() ->
    open_port({spawn_executable, sh()}, [binary, {args, ["-c", "cat"]}]).

%% Port signals by message (io.c:3974-4015), by exit/2 (bif.c:1774-1797),
%% and port_call/3 (erl_bif_port.c:263-320).
signals() ->
    process_flag(trap_exit, true),
    Self = self(),
    Other = spawn(fun() -> receive stop -> ok end end),
    Command = cat(),
    Command ! {Self, {command, <<"hi">>}},
    Commands = flush(),
    spawn(fun() -> Command ! {Self, {command, [<<"a">>, $b]}} end),
    FromOther = flush(),
    Sent = erlang:send(Command, {Self, {command, <<"s">>}}, [noconnect]),
    SendMessages = flush(),
    Command ! {Self, close},
    Close = [flush(), erlang:port_info(Command)],
    Command ! {Self, close},
    Dropped = flush(),
    Bad = [bad_signal(M) || M <- [bad, {Self, bad}, {Self, {command, bad}}, {Other, close},
                                  {Other, {command, <<"x">>}}, {Self, {connect, bad}},
                                  {Other, {connect, Self}}, {Self, {other, x}}, {self, close}]],
    Connect = cat(),
    Connect ! {Self, {connect, Other}},
    Connected = flush(),
    {connected, Owner} = erlang:port_info(Connect, connected),
    {links, Links} = process_info(Self, links),
    ConnectState = [Owner =:= Other, lists:member(Connect, Links), port_close(Connect), flush()],
    Exits = [exit_port(Reason) || Reason <- [normal, kill, boom]],
    ByOther = cat(),
    spawn(fun() -> exit(ByOther, normal) end),
    OtherExit = [flush(), erlang:port_info(ByOther), exit(ByOther, boom)],
    Call = cat(),
    Calls = [catch_error(fun() -> erlang:port_call(Call, 1, x) end),
             catch_error(fun() -> erlang:port_call(Call, x) end)],
    port_close(Call),
    Other ! stop,
    [Commands, FromOther, Sent, SendMessages, Close, Dropped, Bad, Connected, ConnectState,
     Exits, OtherExit, Calls, flush()].

bad_signal(Message) ->
    Port = cat(),
    Port ! Message,
    Result = [flush(), is_list(erlang:port_info(Port))],
    port_close(Port),
    [Result, flush()].

exit_port(Reason) ->
    Port = cat(),
    [exit(Port, Reason), flush(), erlang:port_info(Port)].

catch_error(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

%% The messages in the queue, with each port as the atom port.
flush() ->
    receive Message -> [normal(Message) | flush()] after 100 -> [] end.

normal(Port) when is_port(Port) -> port;
normal({A, B}) -> {normal(A), normal(B)};
normal({A, B, C}) -> {normal(A), normal(B), normal(C)};
normal(Term) -> Term.

%% Open fd ports on the pipes `In1` and `In2` and the file `Out`: read the
%% data and the end of file with `eof`, read the items of port_info/2,
%% close with port_close/1, then read to the end of file without `eof`,
%% which closes the port with the reason normal. The output fd stays open
%% after the first port closes. Then give the errors of open_port/2.
fd(In1, In2, Out) ->
    P1 = open_port({fd, In1, Out}, [binary, eof]),
    true = port_command(P1, <<"ping">>),
    D1 = fd_read(P1),
    I1 = [erlang:port_info(P1, Item) || Item <- [name, input, output, os_pid]],
    C1 = port_close(P1),
    P2 = open_port({fd, In2, Out}, [binary]),
    Ref = erlang:monitor(port, P2),
    true = port_command(P2, <<"pong">>),
    D2 = receive {P2, {data, Data}} -> Data end,
    Down = receive {'DOWN', Ref, port, P2, Reason} -> Reason end,
    Errors = [fd_error(Name, Settings) || {Name, Settings} <-
                  [{{fd, In1}, [binary]}, {{fd, a, Out}, [binary]},
                   {{fd, In1, Out, 0}, [binary]}, {{fd, 1 bsl 59, Out}, [binary]},
                   {{fd, -1, Out}, [binary]}, {{fd, In1, (1 bsl 32) + Out}, [binary]},
                   {{fd, 1 bsl 31, Out}, [binary]}, {{fd, In1, Out}, [binary, exit_status]},
                   {{fd, In1, Out}, [binary, {args, []}]}]],
    {D1, I1, C1, erlang:port_info(P1), D2, Down, erlang:port_info(P2), Errors}.

fd_read(Port) ->
    receive
        {Port, {data, Data}} -> [Data | fd_read(Port)];
        {Port, eof} -> [eof]
    end.

fd_error(Name, Settings) ->
    try open_port(Name, Settings) catch error:Reason -> Reason end.

%% Open two fd ports on the pipe `In`: the second port takes the read
%% select of `In` from the first, so only the second port gets the data
%% and the end of file.
fd_steal(In, Out) ->
    P1 = open_port({fd, In, Out}, [binary]),
    P2 = open_port({fd, In, Out}, [binary, eof]),
    Data = fd_read(P2),
    Stray = receive {P1, Message} -> Message after 100 -> none end,
    {Data, Stray, port_close(P1), port_close(P2)}.
