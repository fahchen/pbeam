%% The timing driver of the benchmark of `benchmark/`.
%%
%% `measure/4` runs `Module:Function(Count)` `Runs` times in the calling
%% process and returns `{Result, Nanoseconds}`: the result of the last run and
%% the minimum time of one run. Each run is timed in the guest with
%% `erlang:monotonic_time(nanosecond)`, so PBeam and ERTS measure the same
%% work in the same way, and the start of the runtime and of the process is
%% part of neither number.
-module(bench_run).

-export([measure/4]).

measure(Module, Function, Count, Runs) ->
    measure(Module, Function, Count, Runs, undefined, infinity).

measure(_Module, _Function, _Count, 0, Result, Best) ->
    {Result, Best};
measure(Module, Function, Count, Runs, _Result, Best) ->
    Start = erlang:monotonic_time(nanosecond),
    Result = Module:Function(Count),
    Time = erlang:monotonic_time(nanosecond) - Start,
    %% An integer is smaller than the atom `infinity` in the term order.
    Next =
        case Time < Best of
            true -> Time;
            false -> Best
        end,
    measure(Module, Function, Count, Runs - 1, Result, Next).
