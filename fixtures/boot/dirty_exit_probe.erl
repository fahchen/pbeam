-module(dirty_exit_probe).
-export([signals/0, cancel/0]).

%% Each function starts prim_tty writer processes on a new TTY of stdout.
%% A writer runs prim_tty:writer_loop/2, which calls write_nif/2 for each
%% {write, From, Iovec} message and sends {Ref, ok} to From after the write
%% (prim_tty.erl:549-567). ERTS runs write_nif/2 on a dirty IO scheduler
%% (prim_tty_nif.c:161). The test does not read stdout until the run ends
%% or until stderr tells it to, so each write waits for the full stdout
%% pipe. The results go to stderr through io:put_chars/2.

%% Send signals to a writer while its write waits, kill it, and write the
%% results to stderr. The run then ends while the write still waits. The
%% timer gives an idle ERTS dirty IO scheduler the time to take the write;
%% the results are the same when the write still waits in the queue.
signals() ->
    TTY = tty(dirty_exit_probe_signals),
    {W, _} = start(TTY, <<"a">>, 20),
    receive after 100 -> ok end,
    Monitor = monitor(process, W),
    {message_queue_len, Length} = process_info(W, message_queue_len),
    W ! hello,
    {message_queue_len, Message} = process_info(W, message_queue_len),
    Self = self(),
    Linked = spawn(fun() -> link(W), Self ! {self(), linked}, receive _ -> ok end end),
    LinkedMonitor = monitor(process, Linked),
    receive {Linked, linked} -> ok end,
    exit(W, normal),
    Alive = is_process_alive(W),
    exit(W, kill),
    Down = receive {'DOWN', Monitor, process, W, Reason} -> Reason end,
    LinkedDown = receive {'DOWN', LinkedMonitor, process, Linked, LinkedReason} -> LinkedReason end,
    Words = [integer_to_list(Length), integer_to_list(Message), atom_to_list(Alive),
             atom_to_list(Down), atom_to_list(LinkedDown)],
    io:put_chars(standard_error, [lists:join(" ", Words), "\n"]).

%% Start 10 writers whose writes wait for the full stdout pipe, then an
%% 11th writer, whose write waits in the queue for a free dirty IO
%% scheduler (erl_init.c:867). Kill the 11th writer. Then a shell writes
%% "killed" to stderr, which does not use a dirty IO scheduler, and the test
%% reads stdout. The 10 writes complete, and the function writes "done".
cancel() ->
    TTY = tty(dirty_exit_probe_cancel),
    Writers = [start(TTY, integer_to_binary(I), 18) || I <- lists:seq(0, 9)],
    {W, _} = start(TTY, <<"X">>, 18),
    Monitor = monitor(process, W),
    exit(W, kill),
    killed = receive {'DOWN', Monitor, process, W, Reason} -> Reason end,
    Port = open_port({spawn_executable, "/bin/sh"},
                     [{args, ["-c", "echo killed >&2"]}, exit_status, binary]),
    receive {Port, {exit_status, 0}} -> ok end,
    [receive {Ref, ok} -> ok end || {_, Ref} <- Writers],
    io:put_chars(standard_error, "done\n").

%% Return a new TTY of stdout. prim_tty:init/1 registers its writer with
%% the registered name of the caller (prim_tty.erl:535, 569-571). The TTY
%% is the first field of the #state{} record of the pinned prim_tty.
tty(Name) ->
    true = register(Name, self()),
    element(2, prim_tty:init(#{input => disabled, output => raw})).

%% Start a writer that writes 2^Doublings copies of the one-byte Binary to
%% TTY. Return when the writer took the message, so its write runs or waits
%% in the queue.
start(TTY, Binary, Doublings) ->
    Ref = make_ref(),
    W = spawn(prim_tty, writer_loop, [TTY, Ref]),
    W ! {write, self(), double([Binary], Doublings)},
    taken(W),
    {W, Ref}.

%% Return an iovec with 2^Doublings copies of the bytes of IoVec. Each step
%% makes the iovec, so no step walks a deep iolist.
double(IoVec, 0) -> IoVec;
double(IoVec, Doublings) -> double(erlang:iolist_to_iovec([IoVec, IoVec]), Doublings - 1).

%% Return when W has no message in its queue: it took the write message.
taken(W) ->
    case process_info(W, message_queue_len) of
        {message_queue_len, 0} -> ok;
        _ -> erlang:yield(), taken(W)
    end.
