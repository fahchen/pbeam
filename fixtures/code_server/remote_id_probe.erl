-module(remote_id_probe).
-compile({nowarn_deprecated_function, [{erlang, phash, 2}]}).
-export([main/0, external/0, link/0, monitor/0, old_monitor/0, old_process_monitor/0, group_leader/0,
         group_leader_leader/0, old_send/0, other_scheduler/0]).

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
         T(fun() -> erlang:send(Po, x) end), lists:member(b@host, nodes(known))],
        signals()
    ]]).

%% The BIFs that take a port or a reference of another node, or an
%% identifier of an old incarnation of the local node, which need no
%% connection, and exit/2,3 with a pid or an alias of a node that is not
%% connected on a node that is not alive (bif.c:225-289, 811-825, 925-935, 1295-1356, 1798-1866,
%% 4914-4964, 5896-5906; erl_hl_timer.c:2245-2250; erl_bif_info.c:1364-1368,
%% 4106-4109; erl_bif_port.c:517-522, 574-579). Each case runs in a new
%% process with trap_exit as given, and gives its result and its messages.
signals() ->
    [RPid, _, _, RPort, _, _, RRef | _] = ids(),
    Old = <<100, 13:16, "nonode@nohost">>,
    OPid = lists:last(ids()),
    OPort = binary_to_term(<<131, 89, Old/binary, 8:32, 7:32>>),
    ORef = binary_to_term(<<131, 90, 1:16, Old/binary, 7:32, 9:32>>),
    S = self(),
    Cases = [
        fun() -> link(RPort) end, fun() -> link(RRef) end,
        fun() -> unlink(RPort) end, fun() -> unlink(RRef) end, fun() -> unlink(RPid) end,
        fun() -> monitor(process, RPort) end, fun() -> monitor(process, RRef) end,
        fun() -> monitor(port, RPort) end, fun() -> exit(RPort, kill) end,
        fun() -> group_leader(RPort, S) end, fun() -> group_leader(S, RPort) end,
        fun() -> group_leader(S, RRef) end, fun() -> group_leader(RRef, S) end,
        fun() -> group_leader(S, OPid) end, fun() -> group_leader(RRef, OPid) end,
        fun() -> erlang:cancel_timer(RRef) end, fun() -> erlang:cancel_timer(RRef, []) end,
        fun() -> erlang:cancel_timer(RRef, [{async, true}]) end,
        fun() -> erlang:cancel_timer(RRef, [{info, false}]) end,
        fun() -> erlang:read_timer(RRef) end, fun() -> erlang:read_timer(RRef, []) end,
        fun() -> erlang:read_timer(RRef, [{async, true}]) end,
        fun() -> unalias(RRef) end, fun() -> demonitor(RRef) end,
        fun() -> erlang:port_info(RPort) end, fun() -> erlang:port_info(RPort, name) end,
        fun() -> is_process_alive(RPid) end, fun() -> process_info(RPid, status) end,
        fun() -> process_info(RPid) end,
        fun() -> link(OPid) end, fun() -> link(OPort) end, fun() -> link(ORef) end,
        fun() -> unlink(OPid) end, fun() -> unlink(OPort) end, fun() -> unlink(ORef) end,
        fun() -> exit(OPid, kill) end, fun() -> exit(OPort, kill) end,
        fun() -> exit(ORef, kill) end, fun() -> exit(ORef, x) end,
        fun() -> erlang:port_info(OPort) end, fun() -> erlang:port_info(OPort, name) end,
        fun() -> erlang:port_info(OPort, bad) end, fun() -> is_process_alive(OPid) end,
        fun() -> process_info(OPid, status) end, fun() -> process_info(OPid, [status, links]) end,
        fun() -> process_info(OPid, bad) end, fun() -> process_info(OPid) end,
        fun() -> monitor(process, OPort) end, fun() -> monitor(port, OPid) end,
        fun() -> monitor(process, ORef) end,
        fun() -> demonitor(ORef) end, fun() -> erlang:cancel_timer(ORef) end,
        fun() -> unalias(ORef) end,
        fun() -> exit(RPid, kill) end, fun() -> exit(RRef, x) end,
        fun() -> erlang:exit(RPid, x, [priority]) end],
    [run(Trap, F) || F <- Cases, Trap <- [false, true]].

run(Trap, F) ->
    Parent = self(),
    Pid = spawn(fun() ->
        process_flag(trap_exit, Trap),
        R = try F() catch C:E -> {C, E} end,
        Parent ! {self(), R, msgs()}
    end),
    receive {Pid, R, M} -> {R, M} end.

msgs() ->
    receive {T, I, V} -> [{T, norm(I), V} | msgs()] after 0 -> [] end.

%% The external term format of each identifier and of a local pid, a
%% local reference, and a local port: the round trip of
%% term_to_binary/1,2 and binary_to_term/1,2, the bytes of the old minor
%% versions with the node as ATOM_EXT (external.c:2877-2960, 3025-3060,
%% 3600-3690, 4677-4890), the tag of each local identifier, the sizes,
%% term_to_iovec/1, and the options safe and used.
external() ->
    Ids = ids(),
    Port = open_port({spawn, "cat"}, []),
    Local = [self(), make_ref(), Port],
    Unknown = <<131, 88, 119, 9, "zq9x@host", 5:32, 0:32, 7:32>>,
    io:format("~p~n", [[
        [binary_to_term(term_to_binary(I)) =:= I || I <- Ids ++ Local],
        [binary_to_term(term_to_binary(I), [safe]) =:= I || I <- Ids ++ Local],
        [term_to_binary(I, [{minor_version, 0}]) || I <- Ids],
        [term_to_binary(I, [{minor_version, 1}, deterministic]) || I <- Ids],
        [binary:at(term_to_binary(I), 1) || I <- Local],
        [erlang:external_size(I) || I <- Ids ++ Local],
        [iolist_to_binary(term_to_iovec(I)) =:= term_to_binary(I) || I <- Ids],
        try binary_to_term(Unknown, [safe]) catch error:badarg -> badarg end,
        element(2, binary_to_term(Unknown, [used])),
        [element(2, binary_to_term(term_to_binary(I, [local]), [used])) || I <- Ids],
        [binary_to_term(term_to_binary(I, [local])) =:= I || I <- Ids],
        zero_numbers()
    ]]),
    port_close(Port).

%% A reference of another node with no numbers: dec_term reads a first
%% number also after the end of the input, and used counts its 4 bytes
%% (external.c:4795-4796, 6217-6241). A term after it starts 4 bytes later,
%% and LOCAL_EXT checks its hash only in decoded_size (external.c:5250-5255).
%% NEW_REFERENCE_EXT rejects a first number of 18 bits (external.c:4780-4783).
zero_numbers() ->
    Zero = <<90, 0:16, 119, 6, "b@host", 5:32>>,
    R = binary_to_term(<<131, Zero/binary>>),
    [element(2, binary_to_term(<<131, Zero/binary, Tail/binary>>, [used]))
     || Tail <- [<<>>, <<1, 2>>, <<1, 2, 3, 4, 5>>]]
    ++ [binary_to_term(term_to_binary(R)) =:= R,
        binary_to_term(term_to_binary({R})) =:= {R},
        try binary_to_term(term_to_binary({R, abc, 1})) catch error:badarg -> badarg end,
        element(2, binary_to_term(<<(term_to_binary({R}, [local]))/binary, 1, 2, 3, 4>>, [used]))]
    ++ [try binary_to_term(<<131, 114, 1:16, 100, 6:16, "b@host", 0, First:32>>) of
            Ref -> node(Ref)
        catch error:badarg -> badarg
        end || First <- [1 bsl 18 - 1, 1 bsl 18]].

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
monitor() -> erlang:monitor(process, remote()).
old_monitor() -> erlang:monitor(port, binary_to_term(<<131, 89, 100, 13:16, "nonode@nohost", 8:32, 7:32>>)).
old_process_monitor() -> erlang:monitor(process, lists:last(ids())).
group_leader() -> erlang:group_leader(self(), remote()).
group_leader_leader() -> erlang:group_leader(remote(), self()).
old_send() -> lists:last(ids()) ! x.
%% A local reference of the second scheduler, which PBeam does not hold.
other_scheduler() ->
    binary_to_term(<<131, 90, 3:16, 100, 13:16, "nonode@nohost", 0:32, 1:32, 2:32, 3:32>>).
