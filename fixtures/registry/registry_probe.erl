-module(registry_probe).
-export([names/0, unregister/0, info/0, send/0, send_error/1, send_to/1, echo/1, wait/0, id/1]).

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

%% `erlang:unregister/1` (bif.c:2324-2335, register.c:429-541) removes the
%% name of a process, and the process can then take a new name. A free name
%% and a term that is not an atom raise badarg. logger_olp:terminate/2
%% (logger_olp.erl:299-302) calls it when a logger handler stops.
unregister() ->
    Self = self(),
    true = register(registry_probe_unregister, Self),
    First = unregister(registry_probe_unregister),
    [First,
     whereis(registry_probe_unregister),
     process_info(Self, registered_name),
     try unregister(registry_probe_unregister) catch error:R1 -> R1 end,
     try unregister(?MODULE:id(1)) catch error:R2 -> R2 end,
     try unregister(?MODULE:id(undefined)) catch error:R3 -> R3 end,
     register(registry_probe_again, Self),
     unregister(registry_probe_again)].

info() ->
    Named = spawn(?MODULE, wait, []),
    true = register(registry_probe_info, Named),
    Unnamed = spawn(?MODULE, wait, []),
    Dead = stopped(spawn(?MODULE, wait, [])),
    [process_info(Named, registered_name),
     process_info(Unnamed, registered_name),
     process_info(Dead, registered_name),
     process_info(Dead, current_stacktrace),
     process_info(Dead, [registered_name]),
     try process_info(Dead, ?MODULE:id(bogus_item)) catch error:R -> R end].

send() ->
    true = register(registry_probe_send, self()),
    Echo = spawn(?MODULE, echo, [self()]),
    true = register(registry_probe_echo, Echo),
    ToSelf = registry_probe_send ! to_self,
    Self = receive to_self -> received end,
    ToEcho = registry_probe_echo ! to_echo,
    Echoed = receive to_echo -> received end,
    ToTuple = {registry_probe_echo, node()} ! to_tuple,
    Tupled = receive to_tuple -> received end,
    Errors = [?MODULE:send_error(To)
              || To <- [registry_probe_missing,
                        {registry_probe_send},
                        {registry_probe_send, node(), extra},
                        {1, node()},
                        {registry_probe_missing, node()},
                        {registry_probe_send, 'registry_probe@other'},
                        {registry_probe_send, 1},
                        1,
                        [],
                        make_ref()]],
    Left = receive Any -> {unexpected, Any} after 0 -> empty end,
    [ToSelf, Self, ToEcho, Echoed, ToTuple, Tupled, Errors, Left].

send_error(To) ->
    try send_to(To) of
        lost -> dropped
    catch
        error:badarg -> badarg
    end.

send_to(To) ->
    To ! lost.

echo(Parent) ->
    receive
        Message ->
            Parent ! Message,
            echo(Parent)
    end.

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
