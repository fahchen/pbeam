%% Measure the distribution output queue before connection setup.
-module(dist_pending_size_probe).
-export([main/0]).

main() ->
    Node = x@nohost,
    Handle = erts_internal:new_connection(Node),
    {ok, _, _, A} = erlang:dist_get_stat(Handle),
    {sink, Node} ! first,
    {ok, _, _, B} = erlang:dist_get_stat(Handle),
    {sink, Node} ! first,
    {ok, _, _, C} = erlang:dist_get_stat(Handle),
    io:format("~w~n", [[A, B, C]]).
