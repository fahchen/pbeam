%% Measure the distribution output busy state of the pinned ERTS.
-module(dist_busy_probe).
-export([main/0]).

main() ->
    Node = busy_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    {ok, Handle} = erts_internal:create_dist_channel(Node, self(), {Flags, 1}),
    Parent = self(),
    Message = binary:copy(<<1>>, 1100000),
    Sender = spawn(fun() -> Parent ! {sent, byte_size(erlang:send({sink, Node}, Message))} end),
    suspended(Sender, 1000),
    {ok, 0, 0, Size} = erlang:dist_get_stat(Handle),
    NoSuspend = erlang:send({sink, Node}, extra, [nosuspend]),
    {ok, 0, 0, After} = erlang:dist_get_stat(Handle),
    _ = erlang:dist_ctrl_get_data(Handle),
    Result = receive {sent, Value} -> Value after 2000 -> timeout end,
    Teardown = teardown(Message),
    io:format("~w~n", [{erlang:system_info(dist_buf_busy_limit), Size >= 1048576,
                         NoSuspend, After =:= Size, Result, Teardown}]).

teardown(Message) ->
    Node = teardown_probe@nohost,
    Handle = erts_internal:new_connection(Node),
    Flags = element(3, erts_internal:get_dflags()),
    Controller = spawn(fun() -> receive stop -> ok end end),
    {message, Ref} = erts_internal:create_dist_channel(Node, Controller, {Flags, 1}),
    receive {Ref, Handle} -> ok after 2000 -> error(controller_timeout) end,
    Parent = self(),
    Sender = spawn(fun() -> Parent ! {teardown_sent, byte_size(erlang:send({sink, Node}, Message))} end),
    suspended(Sender, 1000),
    Controller ! stop,
    receive {teardown_sent, Value} -> Value after 2000 -> timeout end.

suspended(_, 0) -> error(suspend_timeout);
suspended(Sender, Attempts) ->
    case process_info(Sender, status) of
        {status, suspended} -> ok;
        _ -> timer:sleep(1), suspended(Sender, Attempts - 1)
    end.
