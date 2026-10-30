-module(statistics_probe).
-export([main/0, unsupported/1]).

%% Write the form of each item of erlang:statistics/1 that PBeam runs, that
%% the counters do not go down, that each difference is the change of the
%% total since the last call of the item, and the badarg cases
%% (erl_bif_info.c:4160-4325, erl_process.c:12103-12137,
%% erl_time_sup.c:1424-1513).
main() ->
    Pairs = [reductions, exact_reductions, runtime, wall_clock],
    First = [{I, erlang:statistics(I)} || I <- Pairs],
    {C1, 0} = erlang:statistics(context_switches),
    work(200000),
    timer:sleep(20),
    Second = [{I, erlang:statistics(I)} || I <- Pairs],
    {C2, Zero} = erlang:statistics(context_switches),
    {G1, G2, G3} = erlang:statistics(garbage_collection),
    T = fun(F) -> try F() catch C:E -> {C, E} end end,
    io:format("~p~n", [[
        [{I, is_integer(A) andalso is_integer(B) andalso A >= 0 andalso B >= 0 andalso B =< A}
         || {I, {A, B}} <- First],
        [{I, A2 >= A1, B2 =:= A2 - A1}
         || {{I, {A1, _}}, {I, {A2, B2}}} <- lists:zip(First, Second)],
        [{I, A2 > A1} || {{I, {A1, _}}, {I, {A2, _}}} <- lists:zip(First, Second),
                         I =/= runtime],
        element(2, element(2, lists:keyfind(wall_clock, 1, Second))) >= 20,
        [is_integer(C1), C2 >= C1, Zero],
        [is_integer(G) andalso G >= 0 || G <- [G1, G2]] ++ [G3],
        [erlang:statistics(scheduler_wall_time), erlang:statistics(scheduler_wall_time_all)],
        [T(fun() -> erlang:statistics(I) end)
         || I <- [foo, 1, "runtime", {runtime}, total_active_tasks_al]]
    ]]).

work(0) -> ok;
work(N) -> _ = erlang:phash2(N), work(N - 1).

unsupported(Item) -> erlang:statistics(Item).
