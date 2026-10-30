%% The process scenario of the benchmark of `benchmark/`.
%%
%% Each round spawns one process with `spawn/3`, the process sends one
%% message to the round's caller, and the caller receives it. The benchmark
%% counts one spawn, send, and receive as one unit of work. The caller returns
%% after the last message, so the child of a round sends and then exits while
%% the caller continues with the next round.
-module(bench_spawn).

-export([rounds/1, child/1]).

rounds(0) ->
    ok;
rounds(N) ->
    Self = self(),
    Pid = spawn(?MODULE, child, [Self]),
    receive
        {Pid, done} -> rounds(N - 1)
    end.

child(Parent) ->
    Parent ! {self(), done}.
