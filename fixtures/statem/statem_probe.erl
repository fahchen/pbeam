-module(statem_probe).
-export([main/0, start/1, callback_mode/0, ready/3]).
main() ->
    Server = spawn(?MODULE, start, [self()]),
    Monitor = monitor(process, Server),
    First = ping(Server),
    Second = ping(Server),
    Server ! stop,
    Down = receive {'DOWN', Monitor, process, Server, Reason} -> Reason end,
    {First, Second, Down}.
ping(Server) ->
    Server ! {ping, self()},
    receive {pong, Server, Data} -> Data end.
start(Parent) ->
    put('$ancestors', [self(), Parent]),
    gen_statem:enter_loop(?MODULE, [], ready, 1).
callback_mode() -> state_functions.
ready(info, {ping, From}, Data) ->
    From ! {pong, self(), Data},
    {keep_state, {Data}};
ready(info, stop, _Data) ->
    stop.
