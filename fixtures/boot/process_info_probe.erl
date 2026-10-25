-module(process_info_probe).
-export([links/0, charges/0, messages/0]).

%% Link two named children to a fresh reporter process. The reporter reads
%% the names of its own links with `process_info(self(), links)` and prints
%% them. This process then reads the links of the reporter with
%% `process_info(Reporter, links)` and prints them. A fresh process gives
%% the same link set on the pinned ERTS and on pbeam, which boot with
%% different links.
links() ->
    Parent = self(),
    Reporter = spawn(fun() ->
        A = spawn_link(fun() -> receive _ -> ok end end),
        B = spawn_link(fun() -> receive _ -> ok end end),
        register(probe_a, A),
        register(probe_b, B),
        Parent ! {self_links, names(self())},
        receive stop -> ok end
    end),
    receive {self_links, Self} -> io:format("~p~n", [Self]) end,
    io:format("~p~n", [names(Reporter)]),
    Reporter ! stop,
    receive after 100 -> ok end.

%% Return the registered name of each link of `pid`, or `unnamed`.
names(Pid) ->
    {links, Links} = process_info(Pid, links),
    [name(Link) || Link <- Links].

name(Pid) ->
    case process_info(Pid, registered_name) of
        {registered_name, Name} -> Name;
        [] -> unnamed
    end.

%% Send two atoms to a sleeping child and read the queue of the child and
%% of this process: the child holds [a,b] in arrival order, the caller
%% holds [] (`erl_bif_info.c` item `messages`).
messages() ->
    Child = spawn(fun() -> receive after 10000 -> ok end end),
    Child ! a,
    Child ! b,
    timer:sleep(50),
    io:format("~p~n", [process_info(Child, messages)]),
    io:format("~p~n", [process_info(self(), messages)]).

%% Print `ok` when 2000 reads of a target with 4096 links cost more than
%% 1000 reductions more than 2000 reads of a target with 0 links. The
%% item charges floor(Links / 4) reductions (`erl_bif_info.c:1424,1777`).
%% A charge that is larger than the reductions left in the time slice is
%% not visible in full in the total of the caller, so the probe checks
%% that the charge exists, not its exact total.
charges() ->
    Zero = reporter(0),
    Many = reporter(4096),
    Extra = cost(Many, 2000) - cost(Zero, 2000),
    io:format("~p~n", [case Extra >= 1000 of true -> ok; false -> low end]).

cost(Target, Count) ->
    erlang:yield(),
    {reductions, Before} = process_info(self(), reductions),
    reads(Target, Count),
    erlang:yield(),
    {reductions, After} = process_info(self(), reductions),
    After - Before.

reads(_Target, 0) -> ok;
reads(Target, Count) ->
    _ = process_info(Target, links),
    reads(Target, Count - 1).

%% Spawn a process with `Count` linked children, and wait until the
%% children exist.
reporter(Count) ->
    Parent = self(),
    Pid = spawn(fun() ->
        [spawn_link(fun() -> receive _ -> ok end end) || _ <- lists:seq(1, Count)],
        Parent ! ready,
        receive stop -> ok end
    end),
    receive ready -> Pid end.
