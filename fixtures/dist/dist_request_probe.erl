%% A spawn request on the local node with `erts_internal:spawn_request/4`
%% and `erlang:spawn_request_abandon/1` (bif.c:1078-1208,
%% erl_process.c:12440-12530, 12949-12976).
-module(dist_request_probe).
-export([local/0, abandon/1, boom/0]).

boom() ->
    exit(boom).

%% Print the result class of each local spawn request form. The printed
%% line holds no pid and no reference, so the two runs give the same text.
local() ->
    process_flag(trap_exit, true),
    Reply = messages(spawn_request(lists, seq, [1, 3], [])),
    BadOption = messages(spawn_request(lists, seq, [1, 3], [foo])),
    NotList = class(fun() -> spawn_request(lists, seq, [1, 3], [link | foo]) end),
    BadArguments = class(fun() -> spawn_request(lists, seq, 1, []) end),
    Replies = [reply(Option)
               || Option <- [[{reply, no}], [{reply, yes}], [{reply, error_only}],
                             [{reply, success_only}], [{reply, foo}]]],
    Tag = messages(spawn_request(lists, seq, [1, 3], [{reply_tag, mine}])),
    Monitors = [messages(spawn_request(lists, seq, [1, 3], Option))
                || Option <- [[monitor], [{monitor, [{tag, mine}]}],
                              [{reply, error_only}, monitor]]],
    Linked = abandoned(spawn_request(dist_request_probe, boom, [], [link])),
    Link = {Linked, receive {'EXIT', _, Why} -> Why after 1000 -> none end},
    Abandon = [spawn_request_abandon(make_ref()),
               class(fun() -> spawn_request_abandon(42) end),
               abandoned(spawn_request(timer, sleep, [50], [{reply, no}, monitor]))],
    Internal = [internal(erlang, node, [], []), internal(1, node, [], []),
                internal(erlang, 1, [], []), internal(erlang, node, [], 1),
                internal(erlang, node, [], foo),
                internal(erlang, node, [], [foo]),
                internal(erlang, node, 1, [])],
    io:format("local: ~0p~n", [[{reply, Reply}, {bad_option, BadOption},
                                {not_list, NotList},
                                {bad_arguments, BadArguments},
                                {replies, Replies}, {tag, Tag},
                                {monitors, Monitors}, {link, Link},
                                {abandon, Abandon}, {internal, Internal}]]),
    ok.

%% Print how `spawn_request_abandon/1` treats a request that waits on the
%% server node: the request of a monitor ends, and a request that has no
%% monitor is not pending (`bif.c:1166-1208`).
abandon(Server) ->
    true = net_kernel:connect_node(Server),
    io:format("abandon: ~0p~n",
              [[{monitored, abandoned(spawn_request(Server, timer, sleep, [1000],
                                                    [{reply, error_only}, monitor]))},
                {plain, spawn_request_abandon(spawn_request(Server, timer, sleep,
                                                           [1000], [{reply, no}]))}]]),
    ok.

%% Return the reply of the request with the option list `Option`.
reply(Option) ->
    {Option, messages(spawn_request(lists, seq, [1, 3], Option))}.

%% Return the shape of the reply of the request `Request`, and the reason of
%% its `DOWN` message.
messages(Request) ->
    {shape(catch_message(Request)), down(Request)}.

%% Return whether the request `Request` can be abandoned, and the reason of
%% its `DOWN` message.
abandoned(Request) ->
    {spawn_request_abandon(Request), down(Request)}.

%% Return the reply of the request `Request` within a second.
catch_message(Request) ->
    receive {_, Ref, _, _} = Message when Ref =:= Request -> Message
    after 1000 -> none
    end.

%% Return the reason of the `DOWN` message of the request `Request` within
%% a second.
down(Request) ->
    receive {'DOWN', Ref, _, _, Why} when Ref =:= Request -> Why
    after 1000 -> none
    end.

%% Return the class of the result of `erts_internal:spawn_request/4`.
internal(Module, Function, Arguments, Options) ->
    class(fun() -> erts_internal:spawn_request(Module, Function, Arguments, Options) end).

%% Reduce a reply to its class, so that the line holds no identifier.
shape({Tag, Ref, ok, Pid}) when is_reference(Ref), is_pid(Pid) ->
    {Tag, ok, pid};
shape({Tag, Ref, error, Reason}) when is_reference(Ref) ->
    {Tag, error, Reason};
shape(Message) ->
    Message.

%% Return the class and the `badarg` cause of the call `Fun`.
class(Fun) ->
    try Fun() of
        Value when is_reference(Value) -> {value, reference};
        Value -> {value, Value}
    catch
        error:Reason:Stack -> {error, Reason, cause(Stack)}
    end.

%% Return the `error_info` cause of the first frame of `Stack`.
cause([{_, _, _, [{error_info, Info}]} | _]) -> maps:get(cause, Info, none);
cause(_) -> none.
