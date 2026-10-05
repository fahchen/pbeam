-module(alias_probe).
-export([targets/0, options/0, tags/0, modes/0, downs/0, exits/0, unsupported/1,
         idle/0, relay/0, owner/2, id/1]).

id(X) -> X.

%% A process that waits for stop.
idle() -> receive stop -> ok end.

%% A process that sends messages for the caller, so that no process sends
%% to its own alias (see the row "Signal queue" of docs/erts-differences.md).
relay() ->
    receive
        {send, To, Messages, Caller} -> [To ! M || M <- Messages], Caller ! sent, relay();
        stop -> ok
    end.

via(Relay, To, Messages) ->
    Relay ! {send, To, Messages, self()},
    receive sent -> ok end.

%% A process that makes an explicit_unalias alias, gives it to Parent, and
%% answers ping.
owner(Parent, T) ->
    A = erlang:monitor(process, T, [{alias, explicit_unalias}]),
    Parent ! {alias, self(), A},
    owner_loop(Parent).

owner_loop(Parent) ->
    receive ping -> Parent ! {pong, self()}, owner_loop(Parent) end.

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

%% The options of erlang:monitor/3: erts_monitor_opts (bif.c:702-756) runs
%% before the type check (bif.c:988-997). Each valid call is followed by
%% demonitor/2 with info.
options() ->
    T = spawn(?MODULE, idle, []),
    Monitor = fun(Type, Target, Options) ->
                      t(fun() -> M = erlang:monitor(?MODULE:id(Type), ?MODULE:id(Target), ?MODULE:id(Options)),
                                 erlang:demonitor(M, [info]) end)
              end,
    Opts = [x, [x], [{alias, foo}], [{tag}], [{a, b}], [{alias, demonitor}|x], [{alias, demonitor, x}],
            [{tag, a}|b], [priority, x], [], [{tag, a}, {tag, b}], [{alias, demonitor}, {tag, a}],
            [{alias, explicit_unalias}], [{alias, reply_demonitor}]],
    Results = {[Monitor(process, T, O) || O <- Opts],
               [Monitor(Type, Target, O) || {Type, Target, O} <- [{process, T, [x]}, {foo, T, []}, {foo, T, [x]},
                                                                  {process, 1, []}, {port, T, []},
                                                                  {process, self(), [{alias, demonitor}]}]]},
    T ! stop,
    {Results, flush()}.

%% DOWN messages with the tag option (erl_proc_sig_queue.c:6417-6429),
%% also by name, for noproc, and for a port name.
tags() ->
    T = spawn(?MODULE, idle, []),
    N = spawn(?MODULE, idle, []),
    true = register(alias_probe_named, N),
    M1 = erlang:monitor(process, T, [{tag, my_down}]),
    M2 = erlang:monitor(process, T, [{tag, {t}}, {tag, [1|2]}]),
    M3 = erlang:monitor(process, {alias_probe_named, node()}, [{tag, named}]),
    T ! stop,
    exit(N, boom),
    Downs = [receive {Tag, M, process, Item, Reason} -> {Tag, item(Item, T, N), Reason} end
             || M <- [M1, M2, M3]],
    M4 = erlang:monitor(process, T, [{tag, gone}]),
    M5 = erlang:monitor(process, alias_probe_nobody, [{tag, nobody}]),
    M6 = erlang:monitor(port, alias_probe_nobody, [{tag, p}]),
    Noprocs = [receive {Tag, M, Type, Item, Reason} -> {Tag, Type, item(Item, T, T), Reason} end
               || M <- [M4, M5, M6]],
    {Downs, Noprocs, flush()}.

item(T, T, _) -> t;
item(N, _, N) -> n;
item(Item, _, _) -> Item.

%% The alias modes of erlang:monitor/3, with messages from another process
%% (erl_proc_sig_queue.c:6009-6070; bif.c:437-492, 776-784).
modes() ->
    R = spawn(?MODULE, relay, []),
    T = spawn(?MODULE, idle, []),
    A1 = erlang:monitor(process, T, [{alias, demonitor}]),
    via(R, A1, [a1, a2]),
    D1 = erlang:demonitor(A1, [flush, info]),
    via(R, A1, [a3]),
    A2 = erlang:monitor(process, T, [{alias, reply_demonitor}]),
    via(R, A2, [b1, b2]),
    D2 = erlang:demonitor(A2, [info]),
    A3 = erlang:monitor(process, T, [{alias, explicit_unalias}, {tag, x}]),
    via(R, A3, [c1]),
    D3 = erlang:demonitor(A3, [info]),
    via(R, A3, [c2]),
    D4 = erlang:demonitor(A3, [info]),
    A4 = erlang:monitor(process, self(), [{alias, demonitor}]),
    via(R, A4, [d1]),
    D5 = erlang:demonitor(A4, [info]),
    A5 = erlang:monitor(process, T, [{alias, demonitor}, {alias, reply_demonitor}]),
    via(R, A5, [e1, e2]),
    via(R, make_ref(), [f1]),
    D6 = erlang:demonitor(A5, [info]),
    Messages = flush(),
    T ! stop, R ! stop,
    {[D1, D2, D3, D4, D5, D6], Messages}.

%% The end of an alias at the DOWN message, also for noproc
%% (erl_proc_sig_queue.c:6463-6490).
downs() ->
    R = spawn(?MODULE, relay, []),
    T = spawn(?MODULE, idle, []),
    B1 = erlang:monitor(process, T, [{alias, demonitor}, {tag, my_down}]),
    B2 = erlang:monitor(process, T, [{alias, explicit_unalias}]),
    B3 = erlang:monitor(process, T, [{alias, reply_demonitor}]),
    T ! stop,
    Downs = [receive {Tag, B, process, T, Reason} -> {Tag, Reason} end || B <- [B1, B2, B3]],
    via(R, B1, [g1]), via(R, B2, [g2]), via(R, B3, [g3]),
    Infos = [erlang:demonitor(B, [info]) || B <- [B1, B2, B3]],
    C1 = erlang:monitor(process, T, [{alias, demonitor}]),
    C2 = erlang:monitor(process, T, [{alias, explicit_unalias}]),
    C3 = erlang:monitor(port, alias_probe_nobody, [{alias, reply_demonitor}]),
    Noprocs = [receive {Tag, C, Type, _, Reason} -> {Tag, Type, Reason} end || C <- [C1, C2, C3]],
    via(R, C1, [h1]), via(R, C2, [h2]), via(R, C3, [h3]),
    Messages = flush(),
    R ! stop,
    {Downs, Infos, Noprocs, Messages}.

%% erlang:exit/2 through an alias (bif.c:1750-1772;
%% erl_proc_sig_queue.c:1650-1658, 4659-4672).
exits() ->
    T = spawn(?MODULE, idle, []),
    O = spawn(?MODULE, owner, [self(), T]),
    Mo = erlang:monitor(process, O),
    A = receive {alias, O, Al} -> Al end,
    E1 = exit(A, normal),
    O ! ping,
    Alive = receive {pong, O} -> alive end,
    E2 = exit(A, boom),
    Down = receive {'DOWN', Mo, process, O, R} -> R end,
    Own = erlang:monitor(process, T, [{alias, demonitor}]),
    process_flag(trap_exit, true),
    E3 = exit(Own, hello),
    Trapped = receive {'EXIT', From, hello} -> From =:= self() end,
    E4 = exit(make_ref(), hello),
    true = erlang:demonitor(Own, [info]),
    E5 = exit(Own, gone),
    process_flag(trap_exit, false),
    Own2 = erlang:monitor(process, T, [{alias, demonitor}]),
    E6 = exit(Own2, normal),
    true = erlang:demonitor(Own2, [info]),
    Left = flush(),
    T ! stop,
    {[E1, E2, E3, E4, E5, E6], Alive, Down, Trapped, Left}.

%% Calls that PBeam does not support (see docs/erts-differences.md).
unsupported(priority) -> erlang:monitor(process, spawn(?MODULE, idle, []), [priority]);
unsupported(time_offset) -> erlang:monitor(time_offset, clock_service);
unsupported(own_alias) ->
    A = erlang:monitor(process, spawn(?MODULE, idle, []), [{alias, demonitor}]),
    exit(A, kill).
