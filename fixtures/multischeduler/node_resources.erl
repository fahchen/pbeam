-module(node_resources).
-export([zlib_give/0, zlib_use/0, reference/0, steal_busy/0, steal_idle/0,
         steal_reference/0, steal_worker/1]).

%% Two roots on two schedulers share one zlib stream. The root `zlib_give`
%% opens the stream, gives the control to the root `zlib_use`, and checks
%% that it cannot use the stream after that. The root `zlib_use` deflates
%% with the stream and returns the data that `zlib:uncompress/1` gives back.

-define(WORKERS, 8).
-define(ROUNDS, 200).
-define(DATA, <<"node resources node resources node resources">>).

reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {give, zlib_give()} end),
    _ = spawn(fun() -> Self ! {use, zlib_use()} end),
    Give = receive {give, A} -> A after 60000 -> timeout end,
    Use = receive {use, B} -> B after 60000 -> timeout end,
    {Give, Use}.

zlib_give() ->
    Z = zlib:open(),
    ok = zlib:deflateInit(Z),
    Pid = wait_for(zlib_user, 100000),
    ok = zlib:set_controlling_process(Z, Pid),
    Rejected =
        try zlib:deflate(Z, ?DATA) of
            _ -> used
        catch
            error:not_on_controlling_process -> not_on_controlling_process;
            Class:Reason -> {Class, Reason}
        end,
    Pid ! {zlib, self(), Z},
    receive done -> Rejected after 60000 -> timeout end.

zlib_use() ->
    true = register(zlib_user, self()),
    receive
        {zlib, From, Z} ->
            Deflated = iolist_to_binary(zlib:deflate(Z, ?DATA, finish)),
            ok = zlib:deflateEnd(Z),
            ok = zlib:close(Z),
            From ! done,
            zlib:uncompress(Deflated) =:= ?DATA
    after 60000 -> timeout
    end.

wait_for(_Name, 0) ->
    timeout;
wait_for(Name, Attempts) ->
    case whereis(Name) of
        undefined -> receive after 1 -> wait_for(Name, Attempts - 1) end;
        Pid -> Pid
    end.

%% Two roots on two schedulers. The root `steal_busy` starts all workers on
%% its own scheduler. Each worker uses its zlib stream, yields ?ROUNDS
%% times, reads its scheduler, and uses the stream again. The root returns
%% {ok, Schedulers} when each stream gives back the data, with the sorted
%% schedulers that the workers ran on.
steal_reference() ->
    Self = self(),
    _ = spawn(fun() -> Self ! {busy, steal_busy()} end),
    _ = spawn(fun() -> Self ! {idle, steal_idle()} end),
    Busy = receive {busy, A} -> A after 60000 -> timeout end,
    Idle = receive {idle, B} -> B after 60000 -> timeout end,
    {case Busy of {ok, Ids} when is_list(Ids) -> ok; _ -> Busy end, Idle}.

steal_idle() ->
    true = register(steal_idle, self()),
    receive stop -> ok after 60000 -> timeout end.

steal_busy() ->
    Root = self(),
    Pids = [spawn(?MODULE, steal_worker, [Root]) || _ <- lists:seq(1, ?WORKERS)],
    Result = steal_collect(Pids, []),
    stop_idle(1000),
    Result.

steal_collect([], Ids) ->
    {ok, lists:usort(Ids)};
steal_collect([Pid | Pids], Ids) ->
    receive
        {Pid, true, WorkerIds} -> steal_collect(Pids, WorkerIds ++ Ids);
        {Pid, false, _} -> {unexpected, Pid}
    after 60000 -> timeout
    end.

steal_worker(Root) ->
    Z = zlib:open(),
    ok = zlib:deflateInit(Z),
    First = zlib:deflate(Z, ?DATA),
    Ids = spin(?ROUNDS, []),
    Last = zlib:deflate(Z, <<>>, finish),
    ok = zlib:close(Z),
    Data = zlib:uncompress(iolist_to_binary([First, Last])),
    Root ! {self(), Data =:= ?DATA, Ids}.

spin(0, Ids) ->
    Ids;
spin(Round, Ids) ->
    erlang:yield(),
    spin(Round - 1, [erlang:system_info(scheduler_id) | Ids]).

stop_idle(0) ->
    ok;
stop_idle(Attempts) ->
    case whereis(steal_idle) of
        undefined -> receive after 1 -> stop_idle(Attempts - 1) end;
        Pid -> Pid ! stop, ok
    end.
