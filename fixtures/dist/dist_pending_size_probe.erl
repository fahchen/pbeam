%% Measure the distribution output queue before connection setup.
-module(dist_pending_size_probe).
-export([main/0]).

main() ->
    process_flag(trap_exit, true),
    Send = erts_internal:new_connection(x@nohost),
    {ok, _, _, A} = erlang:dist_get_stat(Send),
    {sink, x@nohost} ! first,
    {ok, _, _, B} = erlang:dist_get_stat(Send),
    {sink, x@nohost} ! first,
    {ok, _, _, C} = erlang:dist_get_stat(Send),
    Link = erts_internal:new_connection(l@nohost),
    true = link(pid(l@nohost)),
    {ok, _, _, L} = erlang:dist_get_stat(Link),
    Monitor = erts_internal:new_connection(m@nohost),
    _ = monitor(process, pid(m@nohost)),
    {ok, _, _, M} = erlang:dist_get_stat(Monitor),
    Spawn = erts_internal:new_connection(s@nohost),
    _ = spawn_request(s@nohost, erlang, node, [], []),
    {ok, _, _, S} = erlang:dist_get_stat(Spawn),
    Exit = erts_internal:new_connection(e@nohost),
    true = exit(pid(e@nohost), boom),
    {ok, _, _, E} = erlang:dist_get_stat(Exit),
    ExitLarge = erts_internal:new_connection(f@nohost),
    true = exit(pid(f@nohost), binary:copy(<<0>>, 1024)),
    {ok, _, _, EL} = erlang:dist_get_stat(ExitLarge),
    io:format("~w~n", [[A, B, C, L, M, S, E, EL]]).

pid(Node) ->
    Name = atom_to_binary(Node),
    binary_to_term(<<131, 88, 118, (byte_size(Name)):16, Name/binary, 1:32, 0:32, 1:32>>).
