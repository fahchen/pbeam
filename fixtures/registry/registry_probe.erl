-module(registry_probe).
-export([names/0, wait/0, id/1]).

names() ->
    Self = self(),
    true = register(registry_probe_names, Self),
    Named = case whereis(registry_probe_names) of
                Self -> named;
                Other -> Other
            end,
    [Named,
     whereis(registry_probe_missing),
     whereis(?MODULE:id(undefined)),
     try whereis(?MODULE:id(1)) catch error:R1 -> R1 end,
     try whereis(?MODULE:id({registry_probe_names})) catch error:R2 -> R2 end,
     freed()].

freed() ->
    Pid = spawn(?MODULE, wait, []),
    true = register(registry_probe_freed, Pid),
    stopped(Pid),
    whereis(registry_probe_freed).

stopped(Pid) ->
    Ref = monitor(process, Pid),
    Pid ! stop,
    receive
        {'DOWN', Ref, process, Pid, normal} -> Pid
    end.

wait() ->
    receive
        stop -> ok
    end.

id(X) -> X.
