-module(remote_id_probe).
-compile({nowarn_deprecated_function, [{erlang, phash, 2}]}).
-export([main/0, link/0, unlink/0, monitor/0, exit/0, group_leader/0, old_send/0]).

%% Decode the pids, ports, and references of another node with
%% binary_to_term/1 (external.c:3133-3200, 4685-4740, 4786-4890) and write
%% node/1, the type tests, the text, the term order, the hashes, the
%% encoding, and the sends of each one (bif.c:2489-2549).
main() ->
    Ids = ids(),
    [P, P1, _, Po, Po1, _, R, _, _, R0, R1, R2, _, _, Old] = Ids,
    Pid0 = binary_to_term(<<131, 88, 100, 13:16, "nonode@nohost", 0:32, 0:32, 0:32>>),
    Local = [self(), make_ref(), binary_to_term(<<131, 120, 100, 13:16, "nonode@nohost", 3:64, 0:32>>)],
    T = fun(F) -> try F() catch C:E -> {C, E} end end,
    Keys = [{K, N} || K <- Ids, N <- [1, 2, 3]],
    Map = maps:from_list([{K, V} || {K, V} <- lists:zip(Keys, lists:seq(1, length(Keys)))]),
    Tab = ets:new(t, [ordered_set]),
    ets:insert(Tab, [{K} || K <- Keys]),
    Set = ets:new(s, [bag]),
    ets:insert(Set, [{I} || I <- Ids]),
    Ms = [{{'$1'}, [{is_pid, '$1'}], [{{'$1', {node, '$1'}}}]},
          {{'$1'}, [{is_port, '$1'}], [{node, '$1'}]},
          {{'$1'}, [{is_reference, '$1'}], [{node, '$1'}]}],
    io:format("~p~n", [[
        [node(I) || I <- Ids],
        [{is_pid(I), is_port(I), is_reference(I), type(I)} || I <- Ids],
        [norm(I) || I <- Ids],
        [norm(I, lists:flatten(io_lib:format("~p", [I]))) || I <- Ids],
        [norm(I) || I <- lists:sort(Ids ++ Local) -- Local],
        [erlang:phash2(I) || I <- Ids],
        [erlang:phash(I, 1 bsl 32) || I <- Ids],
        [term_to_binary(I) || I <- Ids],
        [erts_debug:flat_size(I) || I <- Ids],
        [erts_internal:term_type(I) || I <- Ids],
        [P == P1, P == Po, Po == Po1, Po == Po, R1 == R2, R1 =:= R2, R0 == R1, Old == Pid0, Old > Pid0],
        lists:sort(maps:keys(Map)) =:= lists:sort(Keys),
        [maps:get(K, Map) || K <- lists:sublist(Keys, 5)],
        [[N || {{I, _}} <- ets:select(Tab, [{{{K, '_'}}, [], ['$_']}]), N <- [norm(I)]]
         || K <- [P, Po, R]],
        [norm(I) || I <- ets:select(Tab, [{{{'$1', 2}}, [{'<', '$1', R1}], ['$1']}])],
        [case Out of {I, Node} -> {norm(I), Node}; Node -> Node end
         || Out <- lists:sort(ets:select(Set, Ms))],
        [P ! x, erlang:send(P, y, [noconnect]), erlang:send(R, z, [noconnect]),
         T(fun() -> erlang:send(Po, x) end), lists:member(b@host, nodes(known))]
    ]]).

%% The identifiers: pids, ports, and references of b@host and a@host with
%% each tag, and a pid of an old incarnation of the local node.
ids() ->
    Node = <<100, 6:16, "b@host">>,
    [binary_to_term(B) || B <- [
        <<131, 88, Node/binary, 7:32, 0:32, 5:32>>,
        <<131, 88, Node/binary, 7:32, 1:32, 5:32>>,
        <<131, 103, Node/binary, 9:32, 2:32, 3>>,
        <<131, 120, Node/binary, 7:64, 5:32>>,
        <<131, 89, Node/binary, 8:32, 5:32>>,
        <<131, 102, Node/binary, 9:32, 1>>,
        <<131, 90, 3:16, Node/binary, 5:32, 1:32, 2:32, 3:32>>,
        <<131, 114, 2:16, Node/binary, 2, 4:32, 5:32>>,
        <<131, 90, 5:16, Node/binary, 5:32, 1:32, 2:32, 3:32, 4:32, 5:32>>,
        <<131, 90, 0:16, Node/binary, 5:32, 9:32>>,
        <<131, 90, 1:16, Node/binary, 5:32, 9:32>>,
        <<131, 90, 2:16, Node/binary, 5:32, 9:32, 0:32>>,
        <<131, 88, 100, 6:16, "a@host", 7:32, 0:32, 6:32>>,
        <<131, 90, 4:16, 100, 6:16, "a@host", 6:32, 1:32, 2:32, 3:32, 4:32>>,
        <<131, 88, 100, 13:16, "nonode@nohost", 0:32, 0:32, 7:32>>]].

type(I) when is_pid(I) -> pid;
type(I) when is_port(I) -> port;
type(I) when is_reference(I) -> reference.

%% Write the text of I with the node name in place of the channel number,
%% the atom index of the node name, which depends on the atoms that the run
%% made before, and whether the channel number is 0, as for the local node
%% (erl_node_container_utils.h:109-116).
norm(I) -> norm(I, text(I)).

norm(I, Text) ->
    {Head, Rest} = lists:splitwith(fun(C) -> C < $0 orelse C > $9 end, Text),
    {Channel, Tail} = lists:splitwith(fun(C) -> C >= $0 andalso C =< $9 end, Rest),
    {Head ++ atom_to_list(node(I)) ++ Tail, Channel =:= "0"}.

text(I) when is_pid(I) -> pid_to_list(I);
text(I) when is_port(I) -> port_to_list(I);
text(I) -> ref_to_list(I).

remote() -> hd(ids()).

link() -> erlang:link(remote()).
unlink() -> erlang:unlink(remote()).
monitor() -> erlang:monitor(process, remote()).
exit() -> erlang:exit(remote(), kill).
group_leader() -> erlang:group_leader(remote(), self()).
old_send() -> lists:last(ids()) ! x.
