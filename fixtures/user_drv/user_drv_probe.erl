-module(user_drv_probe).
-export([iovec/0]).

iovec() ->
    [try erlang:iolist_to_iovec(X) catch error:R -> R end || X <- [<<>>, [], foo]].
