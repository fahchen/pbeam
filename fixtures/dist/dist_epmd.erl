%% An `-epmd_module` that registers in no epmd and gives the address of
%% each node as `{ok, Ip, Port, Version}`, as the epmd module of Redbug
%% CLI does. The port is the argument `-dist_epmd_port`.
-module(dist_epmd).
-export([start_link/0, register_node/2, register_node/3, port_please/2,
         port_please/3, address_please/3, names/1]).

start_link() -> ignore.

register_node(Name, Port) -> register_node(Name, Port, inet).

register_node(_Name, _Port, _Family) -> {ok, 1}.

port_please(Name, Host) -> port_please(Name, Host, infinity).

port_please(_Name, _Host, _Timeout) -> noport.

address_please(_Name, Host, Family) ->
    {ok, Ip} = inet:getaddr(Host, Family),
    {ok, [[Port]]} = init:get_argument(dist_epmd_port),
    {ok, Ip, list_to_integer(Port), 6}.

names(_Host) -> {error, address}.
