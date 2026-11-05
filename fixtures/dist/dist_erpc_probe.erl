%% `erpc:call/4,5`, `erpc:multicall/4,5`, `rpc:call/4,5`,
%% `rpc:multicall/4,5`, the `Node.spawn/4` of Elixir (`erlang:spawn/4`), and
%% a remote `code:load_binary/3` through `rpc:call/4`
%% (lib/kernel/src/erpc.erl:277-323, 418-429, 1045-1144,
%% lib/kernel/src/rpc.erl:441-493, 878-893).
-module(dist_erpc_probe).
-export([server/0, client/1, send_back/1]).

server() ->
    io:format("~s~n", [node()]),
    register(esrv, self()),
    receive stop -> erlang:halt() end.

send_back(Client) ->
    Client ! {back, node(), self()},
    ok.

%% Print the result of each call to the server node and to a node that is
%% not a node. The printed line holds the node name of the server only, so
%% the two runs give the same text.
client(Server) ->
    true = net_kernel:connect_node(Server),
    Missing = missing(Server),
    Call = class(fun() -> erpc:call(Server, erlang, node, []) end),
    Timeout = class(fun() -> erpc:call(Server, timer, sleep, [2000], 100) end),
    Error = class(fun() -> erpc:call(Server, erlang, error, [boom]) end),
    Throw = class(fun() -> erpc:call(Server, erlang, throw, [boom]) end),
    Exit = class(fun() -> erpc:call(Server, erlang, exit, [boom]) end),
    NoConnection = class(fun() -> erpc:call(Missing, erlang, node, []) end),
    Multicall = multicall(Server, fun() -> erpc:multicall([Server, node()],
                                                          erlang, node, []) end),
    MulticallTimeout = multicall(Server, fun() -> erpc:multicall([Server, node()],
                                                                 erlang, node, [], 5000) end),
    MulticallThrow = multicall(Server, fun() -> erpc:multicall([Server], erlang,
                                                               throw, [boom]) end),
    MulticallMissing = multicall(Server, fun() -> erpc:multicall([Server, Missing],
                                                                 erlang, node, []) end),
    Rpc = class(fun() -> rpc:call(Server, erlang, node, []) end),
    RpcTimeout = class(fun() -> rpc:call(Server, timer, sleep, [2000], 100) end),
    RpcThrow = class(fun() -> rpc:call(Server, erlang, throw, [boom]) end),
    RpcExit = class(fun() -> rpc:call(Server, erlang, exit, [boom]) end),
    RpcError = stack(rpc:call(Server, erlang, error, [boom])),
    RpcMissing = class(fun() -> rpc:call(Missing, erlang, node, []) end),
    RpcMulticall = multicall(Server, fun() -> rpc:multicall([Server, node()],
                                                            erlang, node, []) end),
    RpcMulticallTimeout = multicall(Server, fun() -> rpc:multicall([Server, node()],
                                                                   erlang, node, [], 5000) end),
    RpcMulticallMissing = multicall(Server, fun() -> rpc:multicall([Server, Missing],
                                                                   erlang, node, []) end),
    Spawn = spawn_on(Server),
    Load = load(Server),
    io:format("client: ~0p~n",
              [normalize(Server,
                         [{erpc_call, Call}, {erpc_timeout, Timeout},
                          {erpc_error, Error}, {erpc_throw, Throw},
                          {erpc_exit, Exit}, {erpc_no_connection, NoConnection},
                          {erpc_multicall, Multicall},
                          {erpc_multicall_timeout, MulticallTimeout},
                          {erpc_multicall_throw, MulticallThrow},
                          {erpc_multicall_missing, MulticallMissing},
                          {rpc_call, Rpc}, {rpc_timeout, RpcTimeout},
                          {rpc_throw, RpcThrow}, {rpc_exit, RpcExit},
                          {rpc_error, RpcError}, {rpc_no_connection, RpcMissing},
                          {rpc_multicall, RpcMulticall},
                          {rpc_multicall_timeout, RpcMulticallTimeout},
                          {rpc_multicall_missing, RpcMulticallMissing},
                          {spawn, Spawn}, {load, Load}])]), ok.

%% Return the name of a node that is not a node of this host.
missing(Server) ->
    list_to_atom("none@" ++ lists:nth(2, string:split(atom_to_list(Server), "@"))).

%% Return the class and the reason of the call `Fun`, without a stack trace
%% of one of the two nodes.
class(Fun) ->
    try Fun() of
        Value -> {value, Value}
    catch
        Class:Reason -> {Class, reason(Reason)}
    end.

reason({exception, Reason, [_ | _]}) -> {exception, Reason, stack};
reason({erpc, Reason}) -> {erpc, Reason};
reason(Reason) -> Reason.

%% Reduce the stack trace of a `badrpc` result of `rpc:call/5`.
stack({badrpc, {'EXIT', {Reason, [_ | _]}}}) -> {badrpc, {'EXIT', {Reason, stack}}};
stack(Value) -> Value.

%% Reduce the result of a multicall: a node name becomes `server` or
%% `local`, so that the line holds no name of one client node.
multicall(Server, Fun) ->
    case Fun() of
        {Results, BadNodes} ->
            {[element_of(Server, Result) || Result <- Results], BadNodes};
        Results ->
            [element_of(Server, Result) || Result <- Results]
    end.

element_of(Server, {ok, Name}) when Name =:= Server -> {ok, server};
element_of(_Server, {ok, Name}) when Name =:= node() -> {ok, local};
element_of(_Server, {ok, Value}) -> {ok, Value};
element_of(Server, Name) when Name =:= Server -> server;
element_of(_Server, Name) when Name =:= node() -> local;
element_of(_Server, {Class, Reason}) -> {Class, reason(Reason)};
element_of(_Server, Value) -> Value.


%% Replace each occurrence of the server atom `Server` with the atom
%% `server`, so that the printed line holds no name of the server node and
%% the ERTS run and the PBeam run give the same text.
normalize(Server, Server) -> server;
normalize(_Server, Value) when not is_tuple(Value), not is_list(Value) -> Value;
normalize(Server, List) when is_list(List) -> [normalize(Server, E) || E <- List];
normalize(Server, Tuple) when is_tuple(Tuple) ->
    list_to_tuple([normalize(Server, E) || E <- tuple_to_list(Tuple)]).

%% Start a process on the server node with `erlang:spawn/4`, the call of the
%% `Node.spawn/4` of Elixir.
spawn_on(Server) ->
    Pid = erlang:spawn(Server, dist_erpc_probe, send_back, [self()]),
    receive
        {back, Node, Pid} -> {node(Pid) =:= Server, Node =:= Server}
    after 5000 ->
        timeout
    end.

%% Load the object code of a module on the server node with a remote
%% `code:load_binary/3` through `rpc:call/4`, and run the module.
load(Server) ->
    {Module, Binary, Filename} = code:get_object_code(dist_erpc_load),
    Loaded = rpc:call(Server, code, load_binary, [Module, Filename, Binary]),
    Run = rpc:call(Server, dist_erpc_load, run, [7]),
    {Loaded, element(1, Run), element(2, Run), element(3, Run) =:= Server}.
