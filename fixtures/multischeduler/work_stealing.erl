-module(work_stealing).
-export([busy/0, idle/0, reference/0, worker/2, spinner/2]).

%% An imbalanced load: the root `busy` starts all workers on its own
%% scheduler, and the root `idle` only waits. Each worker yields, reads its
%% scheduler, sends a tick to the root, and waits for the ack of the tick.
%% The spinner yields and sends ticks without a wait, and the root kills it
%% after its tick ?KILL_AT, so its exit can arrive while it moves. The root
%% `busy` returns {ok, Schedulers} with the sorted schedulers that the
%% workers ran on, and else {unexpected, Reason}.

-define(WORKERS, 8).
-define(ROUNDS, 200).
-define(KILL_AT, 50).

reference() ->
    {A, B} = pair(fun busy/0, fun idle/0),
    [{case A of {ok, Ids} when is_list(Ids) -> ok; _ -> A end, B}].

pair(First, Second) ->
    Self = self(),
    _ = spawn(fun() -> Self ! {first, First()} end),
    _ = spawn(fun() -> Self ! {second, Second()} end),
    A = receive {first, ResultA} -> ResultA after 60000 -> timeout end,
    B = receive {second, ResultB} -> ResultB after 60000 -> timeout end,
    {A, B}.

idle() ->
    true = register(work_stealing_idle, self()),
    receive stop -> ok after 60000 -> timeout end.

busy() ->
    Root = self(),
    Workers = [spawn_monitor(?MODULE, worker, [Root, 1]) || _ <- lists:seq(1, ?WORKERS)],
    {Spinner, _} = spawn_monitor(?MODULE, spinner, [Root, 1]),
    Next = maps:from_list([{Pid, 1} || {Pid, _} <- Workers] ++ [{Spinner, 1}]),
    Result = collect(Spinner, Next, ?WORKERS + 1, []),
    stop_idle(1000),
    Result.

stop_idle(0) ->
    ok;
stop_idle(Attempts) ->
    case whereis(work_stealing_idle) of
        undefined -> receive after 1 -> stop_idle(Attempts - 1) end;
        Pid -> Pid ! stop, ok
    end.

worker(Root, Round) ->
    worker(Root, Round, []).

worker(Root, Round, Ids) when Round > ?ROUNDS ->
    Root ! {ids, self(), Ids};
worker(Root, Round, Ids) ->
    erlang:yield(),
    Id = erlang:system_info(scheduler_id),
    Root ! {tick, self(), Round},
    receive {ack, Round} -> ok end,
    worker(Root, Round + 1, [Id | Ids]).

spinner(Root, Round) ->
    erlang:yield(),
    Root ! {tick, self(), Round},
    spinner(Root, Round + 1).

%% Check that the ticks of each sender arrive in order, that each worker
%% sends its ids before its DOWN, and that each DOWN has the expected reason.
collect(_Spinner, _Next, 0, Ids) ->
    {ok, lists:usort(Ids)};
collect(Spinner, Next, Live, Ids) ->
    receive
        {tick, Pid, Round} ->
            case maps:get(Pid, Next) of
                Round when Pid =:= Spinner, Round =:= ?KILL_AT ->
                    true = exit(Pid, kill),
                    collect(Spinner, Next#{Pid := Round + 1}, Live, Ids);
                Round when Pid =:= Spinner ->
                    collect(Spinner, Next#{Pid := Round + 1}, Live, Ids);
                Round ->
                    Pid ! {ack, Round},
                    collect(Spinner, Next#{Pid := Round + 1}, Live, Ids);
                Expected ->
                    {unexpected, {order, Expected, Round}}
            end;
        {ids, Pid, WorkerIds} ->
            case maps:get(Pid, Next) of
                Round when Round =:= ?ROUNDS + 1 ->
                    collect(Spinner, Next#{Pid := ids}, Live, WorkerIds ++ Ids);
                Other ->
                    {unexpected, {ids, Other}}
            end;
        {'DOWN', _, process, Spinner, killed} ->
            collect(Spinner, Next, Live - 1, Ids);
        {'DOWN', _, process, Pid, normal} when Pid =/= Spinner ->
            case maps:get(Pid, Next) of
                ids -> collect(Spinner, Next, Live - 1, Ids);
                Other -> {unexpected, {down, Other}}
            end;
        {'DOWN', _, process, _, Reason} ->
            {unexpected, {down, Reason}}
    after 60000 ->
        {unexpected, timeout}
    end.
