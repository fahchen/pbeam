%% A module that a node loads with a remote `code:load_binary/3` through
%% `rpc:call/4`, as the Redbug CLI loads object code on another node
%% (lib/kernel/src/rpc.erl:878-893).
-module(dist_erpc_load).
-export([run/1]).

run(Value) ->
    {loaded, Value, node()}.
