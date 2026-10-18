-module(process_info_probe).
-export([links/0]).

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
