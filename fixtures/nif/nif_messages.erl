%% The module of the NIF library fixtures/nif/nif_messages.c: probe/2 and
%% dirty_probe/2 run the process, message, and mutex enif calls of their
%% first argument.
-module(nif_messages).
-export([results/1, mismatches/1, call/3]).

-nifs([probe/2, dirty_probe/2]).

%% Load the library `Path` and return the result of each case, in the
%% order of expected/0.
results(Path) ->
    ok = erlang:load_nif(Path, 0),
    Dead = dead(),
    ToSelf = [probe(0, first), probe(0, {second, [1, 2]})],
    Received = flush(),
    Pids = [probe(1, self()), probe(1, Dead), probe(1, self), probe(1, make_ref()),
            probe(1, 7)],
    Echo = echo(),
    Others = [probe(2, Echo), echoed(), probe(3, echo()), echoed(), probe(2, Dead),
              probe(3, Dead), probe(2, self), flush()],
    Order = [probe(0, a), self() ! b, probe(0, c), flush()],
    Mutexes = [probe(4, 0), probe(5, 0), probe(6, 0)],
    AllocSelf = probe(7, 0),
    Loader = [probe(8, loaded), flush()],
    NoEnv = [probe(10, {no_env, [1]}), flush()],
    Dirty = [dirty_probe(0, 5), flush(), dirty_probe(1, echo()), echoed(),
             dirty_probe(1, Dead), dirty_probe(1, self)],
    [ToSelf, Received, Pids, Others, Order, Mutexes, AllocSelf, Loader, NoEnv, Dirty].

%% Load the library `Path` and return each case whose result is not the
%% result of the pinned ERTS, as {Index, Result, Expected}.
mismatches(Path) ->
    Pairs = lists:zip(results(Path), expected()),
    [{Index, Result, Expected}
     || {Index, {Result, Expected}} <- lists:enumerate(Pairs), Result =/= Expected].

%% Load the library `Path` and return the result of one probe call.
call(Path, Kind, Arg) ->
    ok = erlang:load_nif(Path, 0),
    probe(Kind, Arg).

%% Return the pid of a process that ended.
dead() ->
    {Pid, Ref} = spawn_monitor(fun() -> ok end),
    receive {'DOWN', Ref, process, Pid, normal} -> Pid end.

%% Return a process that sends its first message back as {echo, Message}.
echo() ->
    Parent = self(),
    spawn(fun() -> receive Message -> Parent ! {echo, Message} end end).

%% Return the message of an echo process.
echoed() ->
    receive {echo, Message} -> Message after 1000 -> timeout end.

%% Return the messages in the mailbox, in their order.
flush() ->
    receive Message -> [Message | flush()] after 0 -> [] end.

probe(_Kind, _Arg) -> erlang:nif_error(not_loaded).

dirty_probe(_Kind, _Arg) -> erlang:nif_error(not_loaded).

%% The results of the pinned ERTS (OTP 29.1.1).
expected() ->
    [[1, 1],
     [first, {second, [1, 2]}],
     [1, 0, not_pid, not_pid, not_pid],
     [1, {hello, 42}, 1, {hello, 42}, 0, 0, not_pid, []],
     [1, b, 1, [a, b, c]],
     [ok, ok, ok],
     is_null,
     [1, [loaded]],
     [1, [{no_env, [1]}]],
     [{1, 1}, [{dirty, 5}], 1, {dirty, 0}, 0, not_pid]].
