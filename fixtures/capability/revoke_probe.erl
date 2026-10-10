-module(revoke_probe).
-export([
    cascade/0,
    denied/0,
    exited/0,
    below/0,
    exiting/0,
    released/0,
    launcher/0,
    holder/0,
    done/0
]).

%% Each probe revokes the env capability, so that each holder keeps the
%% process capability that it needs to send its replies. os:getenv/1 shows
%% the state of the env capability of a process.

%% The caller revokes the env capability that it gave to a holder,
%% two times before the holder handles the signals. The revocation goes on
%% to the child of the holder. Both holders stay alive, and the copy of the
%% caller stays.
cascade() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    Before = [call(Worker, getenv), call(Child, getenv)],
    Revoked = [revoke(Worker, env), revoke(Worker, env)],
    After = [call(Worker, getenv), call(Child, getenv), attempt()],
    Alive = [is_process_alive(Worker), is_process_alive(Child)],
    stop([Worker, Child]),
    {Before, Revoked, After, Alive}.

%% Revocations by a process that did not give the copy, and arguments that
%% are not valid. Then the worker revokes the copy that it gave to its
%% child.
denied() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    Results = [
        revoke(Child, env),
        revoke(self(), env),
        call(Worker, {revoke, self(), env}),
        revoke(Worker, file),
        revoke(worker, env),
        call(Worker, {revoke, Child, env}),
        call(Child, getenv),
        call(Worker, getenv)
    ],
    stop([Worker, Child]),
    Results.

%% The worker exits before the revocation. Its delegation record stays, so
%% the revocation still reaches its child.
exited() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    stop_and_wait(Worker),
    Before = call(Child, getenv),
    Revoked = revoke(Worker, env),
    After = call(Child, getenv),
    stop([Child]),
    {Before, Revoked, After}.

%% The child of the worker exits before the revocation. When the worker
%% handles the revocation, it sends it through the record of its child to
%% the grandchild.
below() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    Grandchild = call(Child, child),
    stop_and_wait(Child),
    Revoked = revoke(Worker, env),
    After = [call(Worker, getenv), call(Grandchild, getenv)],
    stop([Worker, Grandchild]),
    {Revoked, After}.

%% The worker exits with the revocation signal in its queue. Its exit sends
%% the revocation on to its child.
exiting() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    Monitor = monitor(process, Worker),
    exit(Worker, kill),
    Revoked = revoke(Worker, env),
    receive
        {'DOWN', Monitor, process, Worker, killed} -> ok
    end,
    After = call(Child, getenv),
    stop([Child]),
    {Revoked, After}.

%% The worker and its child exit. Their copies and records go, so no copy
%% is left to revoke.
released() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    stop_and_wait(Worker),
    stop_and_wait(Child),
    revoke(Worker, env).

%% The worker exits, and then the launcher revokes the env capability of
%% the root process. The root process waits for the revocation; then its
%% read and the read of the child of the worker are denied.
launcher() ->
    Worker = spawn(?MODULE, holder, []),
    Child = call(Worker, child),
    stop_and_wait(Worker),
    Before = [attempt(), call(Child, getenv)],
    Waited = wait_denied(10000),
    After = call(Child, getenv),
    stop([Child]),
    {Before, Waited, After}.

%% A process that answers requests until it gets stop.
holder() ->
    receive
        {getenv, From} ->
            From ! {self(), attempt()},
            holder();
        {child, From} ->
            From ! {self(), spawn(?MODULE, holder, [])},
            holder();
        {{revoke, Holder, Kind}, From} ->
            From ! {self(), revoke(Holder, Kind)},
            holder();
        stop ->
            ok
    end.

call(Pid, Request) ->
    Pid ! {Request, self()},
    receive
        {Pid, Reply} -> Reply
    end.

stop(Pids) ->
    [Pid ! stop || Pid <- Pids],
    ok.

stop_and_wait(Pid) ->
    Monitor = monitor(process, Pid),
    Pid ! stop,
    receive
        {'DOWN', Monitor, process, Pid, normal} -> ok
    end.

revoke(Holder, Kind) ->
    try
        pbeam_cap:revoke(Holder, Kind)
    catch
        error:capability_denied -> denied;
        error:Reason -> {error, Reason}
    end.

attempt() ->
    try os:getenv("PBEAM_REVOKE_PROBE") of
        _ -> ok
    catch
        error:capability_denied -> denied
    end.

wait_denied(0) ->
    timeout;
wait_denied(Count) ->
    receive
    after 1 -> ok
    end,
    case attempt() of
        denied -> denied;
        ok -> wait_denied(Count - 1)
    end.

done() -> ok.
