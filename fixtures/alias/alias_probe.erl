-module(alias_probe).
-export([targets/0, unsupported/1, idle/0, id/1]).

id(X) -> X.

%% A process that waits for stop.
idle() -> receive stop -> ok end.

flush() -> receive M -> [M | flush()] after 0 -> [] end.

t(F) -> try {ok, F()} catch C:E -> {C, E} end.

%% erlang:monitor/2 (bif.c:758-986): the type, the target, a monitor of
%% the caller, which ERTS does not create (bif.c:781-784), and the item of
%% the DOWN message for a pid, a name, and a port name.
targets() ->
    T = spawn(?MODULE, idle, []),
    N = spawn(?MODULE, idle, []),
    true = register(alias_probe_named, N),
    true = register(alias_probe_self, self()),
    Errors = [t(fun() -> erlang:monitor(?MODULE:id(Type), ?MODULE:id(Target)) end)
              || {Type, Target} <- [{foo, T}, {1, T}, {process, 1}, {process, {a, b, c}},
                                    {process, {a, 'b@c'}}, {process, {a, 1}}, {process, {1, node()}},
                                    {process, make_ref()}, {port, T}, {port, {a, b, c}},
                                    {port, {a, 'b@c'}}, {time_offset, foo}]],
    Self = [erlang:demonitor(erlang:monitor(process, ?MODULE:id(S)), [info])
            || S <- [self(), alias_probe_self, {alias_probe_self, node()}]],
    M1 = erlang:monitor(process, T),
    M2 = erlang:monitor(process, alias_probe_named),
    M3 = erlang:monitor(process, {alias_probe_named, node()}),
    T ! stop,
    exit(N, boom),
    Downs = [receive {'DOWN', M, process, Item, Reason} -> {item(Item, T, N), Reason} end
             || M <- [M1, M2, M3]],
    M4 = erlang:monitor(process, T),
    M5 = erlang:monitor(process, alias_probe_nobody),
    M6 = erlang:monitor(port, alias_probe_nobody),
    M7 = erlang:monitor(port, {alias_probe_nobody, node()}),
    Noprocs = [receive {'DOWN', M, Type, Item, Reason} -> {Type, item(Item, T, T), Reason} end
               || M <- [M4, M5, M6, M7]],
    Infos = [erlang:demonitor(M, [info]) || M <- [M1, M2, M4, M6]],
    {Errors, Self, Downs, Noprocs, Infos, flush()}.

item(T, T, _) -> t;
item(N, _, N) -> n;
item(Item, _, _) -> Item.

%% Calls that PBeam does not support (see docs/erts-differences.md).
unsupported(time_offset) -> erlang:monitor(time_offset, clock_service).
