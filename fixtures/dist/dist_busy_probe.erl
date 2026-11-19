%% Compare distribution sender suspension and resumption with pinned ERTS.
-module(dist_busy_probe).
-export([main/0, nosuspend/0]).

main() ->
    Node = busy_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    Parent = self(),
    Chunk = binary:copy(<<1>>, 600000),
    ok = erlang:send({sink, Node}, Chunk, [nosuspend]),
    Sender = spawn(fun() -> Parent ! {sent, erlang:send({sink, Node}, Chunk, [])} end),
    suspended(Sender, 1000),
    {ok, 0, 0, Size} = erlang:dist_get_stat(Handle),
    NoSuspend = erlang:send({sink, Node}, extra, [nosuspend]),
    {ok, 0, 0, After} = erlang:dist_get_stat(Handle),
    First = erlang:dist_ctrl_get_data(Handle),
    {status, HalfStatus} = process_info(Sender, status),
    Second = erlang:dist_ctrl_get_data(Handle),
    Result = receive {sent, Value} -> Value after 2000 -> timeout end,
    Empty = erlang:dist_ctrl_get_data(Handle),
    Message = binary:copy(<<1>>, 1100000),
    Teardown = teardown(Message),
    Pending = pending_abort(Message),
    Async = async_dist(Message),
    io:format("~w~n", [{Size >= 1048576, NoSuspend, After =:= Size,
                         is_list(First), HalfStatus, is_list(Second), Empty, Result, Teardown, Pending, Async}]).

teardown(Message) ->
    Node = teardown_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    Controller = spawn(fun() -> receive stop -> ok end end),
    {message, Ref} = erts_internal:create_dist_channel(Node, Controller, {Flags, 1}),
    receive {Ref, Handle} -> ok after 2000 -> error(controller_timeout) end,
    Parent = self(),
    Sender = spawn(fun() -> Parent ! {teardown_sent, ({sink, Node} ! Message) =:= Message} end),
    suspended(Sender, 1000),
    Controller ! stop,
    receive {teardown_sent, Value} -> Value after 2000 -> timeout end.

pending_abort(Message) ->
    Node = pending_suspend_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Parent = self(),
    Sender = spawn(fun() -> Parent ! {pending_sent, erlang:send({sink, Node}, Message, [])} end),
    suspended(Sender, 1000),
    true = erts_internal:abort_pending_connection(Node, Handle),
    receive {pending_sent, Value} -> Value after 2000 -> timeout end.

async_dist(Message) ->
    Parent = self(),
    spawn(fun() -> Parent ! {async_result, async_controller(Message)} end),
    receive {async_result, Value} -> Value after 2000 -> timeout end.

async_controller(Message) ->
    Node = async_busy_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Pending = async_send(Node, Message),
    PendingNext = erlang:send({sink, Node}, extra, [nosuspend]),
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    PromotedNext = erlang:send({sink, Node}, extra, [nosuspend]),
    drain(Handle),
    Connected = async_send(Node, Message),
    ConnectedNext = erlang:send({sink, Node}, extra, [nosuspend]),
    Packet = erlang:dist_ctrl_get_data(Handle),
    {Pending, PendingNext, PromotedNext, Connected, ConnectedNext, is_list(Packet)}.

async_send(Node, Message) ->
    Parent = self(),
    spawn(fun() ->
        process_flag(async_dist, true),
        Parent ! {async_sent, erlang:send({sink, Node}, Message, [])}
    end),
    receive {async_sent, Value} -> Value after 2000 -> timeout end.

nosuspend() ->
    Node = nosuspend_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    Message = binary:copy(<<1>>, 1100000),
    First = erlang:send({sink, Node}, Message, [nosuspend]),
    {ok, 0, 0, Before} = erlang:dist_get_stat(Handle),
    Second = erlang:send({sink, Node}, extra, [nosuspend]),
    {ok, 0, 0, After} = erlang:dist_get_stat(Handle),
    drain(Handle),
    Third = erlang:send({sink, Node}, after_drain, [nosuspend]),
    Mid = hysteresis(),
    Pending = pending(),
    Boundaries = [boundary(Size) || Size <- [524287, 524288, 524289]],
    io:format("~w~n", [{First, Before >= 1048576, Second, Before =:= After, Third, Mid, Pending, Boundaries}]).

boundary(Size) ->
    Parent = self(),
    spawn(fun() -> Parent ! {boundary_result, boundary_controller(Size)} end),
    receive {boundary_result, Result} -> Result after 2000 -> timeout end.

boundary_controller(Target) ->
    Node = boundary_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    ok = erlang:send({sink, Node}, binary:copy(<<1>>, 1024), [nosuspend]),
    {ok, _, _, Calibration} = erlang:dist_get_stat(Handle),
    _ = erlang:dist_ctrl_get_data(Handle),
    Overhead = Calibration - 1024,
    ok = erlang:send({sink, Node}, binary:copy(<<1>>, 600000), [nosuspend]),
    ok = erlang:send({sink, Node}, binary:copy(<<1>>, Target - Overhead), [nosuspend]),
    _ = erlang:dist_ctrl_get_data(Handle),
    {ok, _, _, Size} = erlang:dist_get_stat(Handle),
    Result = erlang:send({sink, Node}, extra, [nosuspend]),
    {Size, Result}.

pending() ->
    Node = pending_busy_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    ok = erlang:send({sink, Node}, binary:copy(<<3>>, 1100000), [nosuspend]),
    {ok, _, _, Before} = erlang:dist_get_stat(Handle),
    Result = erlang:send({sink, Node}, extra, [nosuspend]),
    {ok, _, _, After} = erlang:dist_get_stat(Handle),
    true = erts_internal:abort_pending_connection(Node, Handle),
    {Result, Before =:= After}.

hysteresis() ->
    Parent = self(),
    spawn(fun() -> Parent ! {hysteresis_result, hysteresis_controller()} end),
    receive {hysteresis_result, Result} -> Result after 2000 -> timeout end.

hysteresis_controller() ->
    Node = hysteresis_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    Message = binary:copy(<<2>>, 600000),
    ok = erlang:send({sink, Node}, Message, [nosuspend]),
    ok = erlang:send({sink, Node}, Message, [nosuspend]),
    _ = erlang:dist_ctrl_get_data(Handle),
    {ok, _, _, Size} = erlang:dist_get_stat(Handle),
    Result = erlang:send({sink, Node}, extra, [nosuspend]),
    {Size >= 524288 andalso Size < 1048576, Result}.

drain(Handle) ->
    case erlang:dist_ctrl_get_data(Handle) of
        none -> ok;
        _ -> drain(Handle)
    end.

suspended(_, 0) -> error(suspend_timeout);
suspended(Sender, Attempts) ->
    case process_info(Sender, status) of
        {status, suspended} -> ok;
        _ -> timer:sleep(1), suspended(Sender, Attempts - 1)
    end.
