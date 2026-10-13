%% The release of a large graph of N cells, for row 11 of
%% docs/term-performance-design-codex-v2.md and task #559. Each function
%% builds the graph in a live process and lets the runtime release it. The
%% observable results are the same on pinned ERTS.
-module(release_probe).

-export([exits_with/1, shared/1, shutdown/1, sender/2, holder/2]).

%% Build a list of N cells in the caller, keep it in the process dictionary,
%% and return `ok`. The exit of the caller releases the graph.
exits_with(N) ->
    put(graph, build(N)),
    ok.

%% Spawn a process that builds a list of N cells and sends it to the caller.
%% Wait for the message and for the end of the sender, and return the length
%% of the graph. The send shares the graph, so the exit of the sender
%% releases one holder and the exit of the caller releases the last one.
shared(N) ->
    Self = self(),
    {Pid, Reference} = spawn_opt(?MODULE, sender, [Self, N], [monitor]),
    Graph =
        receive
            {graph, Value} -> Value
        end,
    receive
        {'DOWN', Reference, process, Pid, normal} -> ok
    end,
    length(Graph).

%% Spawn a process that builds a list of N cells and waits. Return `ok` after
%% the child built the graph, so that the run ends with the child alive and
%% the drop of the scheduler releases the graph of the child.
shutdown(N) ->
    Self = self(),
    _ = spawn_opt(?MODULE, holder, [Self, N], []),
    receive
        ready -> ok
    end.

%% Send a list of N cells to `Self`.
sender(Self, N) ->
    Self ! {graph, build(N)}.

%% Build a list of N cells, tell `Self`, and wait without an end.
holder(Self, N) ->
    put(graph, build(N)),
    Self ! ready,
    receive
        never -> ok
    end.

%% Return a list of N cells.
build(N) ->
    build(N, []).

build(0, Acc) ->
    Acc;
build(N, Acc) ->
    build(N - 1, [N | Acc]).
