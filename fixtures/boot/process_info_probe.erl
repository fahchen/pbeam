-module(process_info_probe).
-export([links/0, charges/0, messages/0, items/0, wait/0, stacktrace/0, nested/0]).

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

%% Read the items of a sleeping child that `process_info/1` also reports,
%% from another process: the child waits with an empty dictionary, a false
%% `trap_exit`, the default error handler, its current function, and its
%% start MFA. The values come from the same item values as
%% `process_info/1`, so a read of another process agrees with it.
items() ->
    Child = spawn(process_info_probe, wait, []),
    timer:sleep(50),
    [io:format("~p~n", [process_info(Child, Item)]) ||
        Item <- [status, message_queue_len, dictionary, trap_exit,
                 error_handler, current_function, initial_call, priority]],
    exit(Child, kill).

%% process_info/2 with lists of items and current_stacktrace, as the crash
%% report of gen_server reads the client (gen_server.erl:2804): another
%% process gives the frame where it waits first, the caller gives its frames
%% without process_info/2 (erl_bif_info.c:2396-2461), and a list gives
%% {registered_name, []} for a process without a name and undefined for a
%% dead process (erl_bif_info.c:1134-1145, 1616-1625).
stacktrace() ->
    Parent = self(),
    Child = spawn(process_info_probe, nested, []),
    Named = spawn(process_info_probe, wait, []),
    register(process_info_probe_named, Named),
    Dead = spawn(fun() -> ok end),
    Self = spawn(fun() -> Parent ! {self, own(3)} end),
    timer:sleep(50),
    Busy = spawn(process_info_probe, nested, []),
    Busy ! hello,
    [io:format("~p~n", [Result]) || Result <- [
        process_info(Child, current_stacktrace),
        process_info(Child, [current_stacktrace, registered_name]),
        process_info(Named, [registered_name, current_stacktrace, status]),
        process_info(Named, [current_function, registered_name]),
        process_info(Dead, [current_stacktrace, registered_name]),
        process_info(Busy, [current_stacktrace, registered_name]),
        receive {self, Own} -> Own end
    ]],
    [exit(P, kill) || P <- [Child, Named, Busy, Self]].

%% Read the own frames three calls deep.
own(0) ->
    Info = process_info(self(), [registered_name, current_stacktrace, current_function]),
    {Info};
own(N) ->
    {Result} = own(N - 1),
    {Result}.

%% Wait in a function that a call of nested/0 left on the stack.
nested() ->
    deeper(),
    ok.

deeper() ->
    receive after 10000 -> ok end,
    ok.

%% Wait for one message, so a reader of another process sees a waiting
%% process in a known function.
wait() ->
    receive after 10000 -> ok end.

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
