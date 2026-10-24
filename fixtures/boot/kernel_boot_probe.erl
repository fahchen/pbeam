-module(kernel_boot_probe).
-compile(nowarn_deprecated_catch).
-behaviour(application).
-behaviour(supervisor).
-export([main/0, main/1, lookup/0, gethost/0, supports/0, socket/0, sockopt/0, socket_select/0, socket_io/0,
         socket_dgram/0,         socket_acceptors/0, socket_connect_invalid/0, socket_recvfrom_flags/0, socket_recvfrom_local_flags/0,
         socket_recvfrom_errqueue/0, socket_sendv/0, socket_backend_tcp/0, socket_backend_udp/0, sockopt_otp/0, sockopt_host/0, start/2, stop/1, init/1]).

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% main -kernel logger_level warning` runs main/0 in the boot process of init after the boot script
%% starts kernel and stdlib (init.erl:1539-1571). main/0 displays the
%% kernel processes, the started applications, and the results of
%% application:start/1, ensure_all_started/1,2, and stop/1 for the
%% applications of kernel_boot_app.app and kernel_boot_dep.app in
%% fixtures/boot. It then calls init:stop/0, and main/1 calls
%% init:stop/1 with its status. init stops application_controller, which
%% stops each running application (application_controller.erl:1263-1300),
%% so stop/1 of kernel_boot_app displays its state, and init halts with
%% the status (init.erl:924-948). `mise run fixture-beam` compiles this
%% file.

main() ->
    report(),
    init:stop().

main([Status]) ->
    report(),
    init:stop(list_to_integer(atom_to_list(Status))).

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% lookup -kernel logger_level warning` without a domain in an inetrc file
%% runs lookup/0. On Linux, inet_config:init/0 looked up the host name at
%% the kernel start (inet_config.erl:110-133). lookup/0 displays the domain
%% after that, and the results of the native lookup of the host name and of
%% localhost through the inet_gethost port program
%% (inet_gethost_native.erl:412-428), and then calls init:stop/0.
lookup() ->
    Name = inet_db:gethostname(),
    erlang:display({lookup, inet_db:res_option(lookup), inet_db:res_option(domain),
                    inet:gethostbyname(Name), inet:gethostbyname("localhost"),
                    is_pid(whereis(inet_gethost_native))}),
    init:stop().

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% gethost -kernel logger_level warning` runs gethost/0. It displays the
%% replies of the inet_gethost port program (inet_gethost.c:180-249) to
%% requests that it answers, discards, or ends on, and the results of
%% inet_gethost_native and inet for localhost, addresses, and an unknown
%% name, and then calls init:stop/0.
gethost() ->
    Ask = fun(Requests, Timeout) ->
                  Port = gethost_port(),
                  [begin true = port_command(Port, R),
                         receive {Port, M} -> M after Timeout -> none end
                   end || R <- Requests]
          end,
    Discarded = Ask([<<1:32, 3>>, <<16#FFFFFFFF:32, 4, 0, 0:32>>, <<2:32, 9, 1>>,
                     <<3:32, 1, 7, "x", 0>>, <<4:32, 1, 1, "localhost">>,
                     <<5:32, 2, 1, 1, 2, 3>>], 300),
    Answered = Ask([<<6:32, 1, 1, "localhost", 0>>, <<7:32, 2, 2, 0:120, 1>>,
                    <<8:32, 2, 1, 127, 0, 0, 1>>,
                    <<9:32, 1, 2, "no-such-host.invalid", 0>>,
                    <<10:32, 2, 2, 16#20010db8:32, 0:96>>], 20000),
    Drain = fun Drain(Port, Timeout) ->
                    receive {Port, M} -> [M | Drain(Port, 1000)]
                    after Timeout -> []
                    end
            end,
    Fatal = [begin Port = gethost_port(), true = port_command(Port, R), Drain(Port, 20000) end
             || R <- [<<1, 2, 3>>, <<1:32, 4, 0, 1:32>>]],
    Native = [inet_gethost_native:gethostbyname("localhost"),
              inet_gethost_native:gethostbyname("localhost", inet6),
              inet_gethost_native:gethostbyname("127.0.0.1"),
              inet_gethost_native:gethostbyname("no-such-host.invalid"),
              inet_gethost_native:gethostbyaddr("127.0.0.1"),
              inet_gethost_native:gethostbyaddr("::1"),
              inet:gethostbyname("localhost")],
    erlang:display({gethost, Discarded, Answered, Fatal, Native}),
    init:stop().

gethost_port() ->
    {ok, [BinDir]} = init:get_argument(bindir),
    open_port({spawn_executable, filename:join(BinDir, "inet_gethost")},
              [{args, ["4"]}, {packet, 4}, eof, exit_status, binary]).

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% supports -kernel logger_level warning` runs supports/0. erl_init:start/2
%% ran prim_socket:on_load/0, which loads the NIF library and stores the
%% tables of nif_supports/1 as persistent terms (erl_init.erl:42-48,
%% prim_socket.erl:137-204). supports/0 displays the result of
%% nif_supports/0, each table as a sorted list, and the results of other
%% keys, and then calls init:stop/0.
supports() ->
    Tables = [{Key, lists:sort(maps:to_list(persistent_term:get({prim_socket, Key})))}
              || Key <- [protocols, options, ioctl_requests, ioctl_flags, msg_flags]],
    erlang:display({supports, prim_socket:supports(), Tables,
                    prim_socket:supports(other), prim_socket:supports("options"),
                    prim_socket:is_supported(local), prim_socket:is_supported(other),
                    prim_socket:is_supported(options, {socket, reuseaddr}),
                    prim_socket:is_supported(other, local)}),
    init:stop().

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% socket -kernel logger_level warning` runs socket/0. It opens, binds,
%% and closes sockets with socket:open/2,3,4, socket:bind/2,
%% socket:sockname/1, and socket:close/1, and displays socket:info/0,1 and
%% the error returns. It replaces each socket, pid, and port number that
%% differs between runs with an atom or a boolean. It displays only the
%% results that are the same on Linux and macOS.
socket() ->
    process_flag(trap_exit, true),
    Owner = fun(Info) -> maps:update_with(owner, fun(P) -> P =:= self() end, Info) end,
    Info0 = socket:info(),
    {ok, S} = socket:open(inet, stream, tcp),
    Opened = Owner(socket:info(S)),
    Bound = socket:bind(S, #{family => inet, addr => loopback, port => 0}),
    {ok, #{family := inet, addr := {127, 0, 0, 1}, port := Port}} = socket:sockname(S),
    Listed = lists:member(S, socket:which_sockets()),
    During = maps:get(counters, socket:info()),
    BoundInfo = maps:get(rstates, socket:info(S)),
    {_, Ref} = S,
    Early = (catch prim_socket:finalize_close(Ref)),
    Closed = socket:close(S),
    After = Owner(socket:info(S)),
    Again = [socket:close(S), socket:sockname(S),
             socket:bind(S, #{family => inet, addr => loopback}),
             prim_socket:finalize_close(Ref), lists:member(S, socket:which_sockets())],
    {ok, D} = socket:open(inet6, dgram, #{use_registry => false}),
    Wrap = socket:bind(D, #{family => inet6, addr => loopback, port => 65536}),
    {ok, #{family := inet6, addr := {0, 0, 0, 0, 0, 0, 0, 1}, port := Port6,
           flowinfo := 0, scope_id := 0}} = socket:sockname(D),
    Unlisted = lists:member(D, socket:which_sockets()),
    {_, DRef} = D,
    ok = prim_socket:close(DRef),
    ok = prim_socket:finalize_close(DRef),
    {ok, U} = socket:open(local, stream),
    Path = list_to_binary("/tmp/pbeam_socket_probe_" ++ os:getpid()),
    _ = file:delete(Path),
    Local = [socket:sockname(U), socket:bind(U, #{family => local, path => Path}),
             socket:sockname(U) =:= {ok, #{family => local, path => Path}},
             maps:get(domain, socket:info(U))],
    ok = socket:close(U),
    _ = file:delete(Path),
    {ok, B} = socket:open(inet, dgram),
    BindErrors = [socket:bind(B, A) || A <- [#{family => inet, addr => {1, 2, 3}},
                                             #{family => inet, port => 1 bsl 40},
                                             #{family => inet, addr => {256, 0, 0, 1}},
                                             #{family => inet6, addr => {1, 2}},
                                             #{family => local, path => binary:copy(<<"a">>, 200)},
                                             #{family => inet, addr => {1, 2, 3, 4}},
                                             #{family => foo}, #{}, foo]],
    BindMore = [socket:bind(B, A) || A <- [#{family => inet, addr => broadcast},
                                           #{family => inet, addr => foo},
                                           #{family => inet, port => foo}]],
    TypeB = maps:get(type, socket:info(B)),
    ok = socket:close(B),
    {ok, B6} = socket:open(inet6, dgram),
    BindMore6 = [socket:bind(B6, A) || A <- [#{family => inet6, addr => foo},
                                             #{family => inet6, addr => {0, 0, 0, 0, 0, 0, 0, 1}},
                                             #{family => inet6, addr => any}]],
    ok = socket:close(B6),
    Native = [native_socket(socket:open(2, 2)), native_socket(socket:open(inet, 2)),
              socket:open(inet, stream, 17),
              native_socket(socket:open(local, seqpacket)),
              native_socket(socket:open(16, raw, 0)),
              native_socket(socket:open(16, dgram, 0))],
    {ok, A} = socket:open(local, dgram),
    Abstract = [socket:bind(A, #{family => local, path => <<0, "pbeam_socket_probe">>}),
                socket:sockname(A)],
    ok = socket:close(A),
    OpenErrors = [socket:open(foo, stream), socket:open(inet, foo),
                  socket:open(inet, stream, foo),
                  catch socket:open(inet, stream, 1 bsl 40),
                  socket:open(1 bsl 40, stream), socket:open(inet, 1 bsl 40)],
    Badargs = [catch prim_socket:info(make_ref()), catch prim_socket:bind(make_ref(), #{family => inet}),
               catch prim_socket:sockname(foo), catch prim_socket:close(make_ref()),
               catch prim_socket:finalize_close(make_ref()), catch prim_socket:finalize_close(foo)],
    Parent = self(),
    {Child, MRef} = spawn_monitor(fun() -> {ok, C} = socket:open(inet, stream, tcp), Parent ! {child, C} end),
    Down = receive {child, C} -> C end,
    receive {'DOWN', MRef, process, Child, normal} -> ok end,
    {Closer, CRef} = spawn_monitor(fun() -> {ok, U2} = socket:open(inet, dgram, udp), ok = socket:close(U2) end),
    receive {'DOWN', CRef, process, Closer, normal} -> ok end,
    {ok, K} = socket:open(inet, stream, tcp),
    {_, KRef} = K,
    {Killer, KMRef} = spawn_monitor(fun() -> ok = prim_socket:close(KRef) end),
    receive {'DOWN', KMRef, process, Killer, normal} -> ok end,
    CloserDown = [maps:get(counters, socket:info()), lists:member(K, socket:which_sockets()),
                  maps:get(rstates, socket:info(K)), socket:close(K)],
    OwnerDown = [CloserDown, maps:get(counters, socket:info()),Owner(maps:remove(counters, socket:info(Down))),
                 lists:member(Down, socket:which_sockets()), socket:close(Down)],
    erlang:display({socket, OwnerDown, Info0, Opened, Bound, Port > 0, Listed, During, BoundInfo,
                    element(1, element(2, Early)), Closed, After, Again, Wrap, Port6 > 0,
                    Unlisted, Local, BindErrors, BindMore, TypeB, BindMore6, Native, Abstract,
                    [strip(E) || E <- OpenErrors], [strip(E) || E <- Badargs],
                    maps:get(counters, socket:info())}),
    init:stop().

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% sockopt -kernel logger_level warning` runs sockopt/0. It sets and gets
%% socket options with socket:setopt/3 and socket:getopt/2,3, binds
%% sockets to the atoms any, loopback, and broadcast, and displays the
%% results and the error returns.
sockopt() ->
    process_flag(trap_exit, true),
    {ok, S} = socket:open(inet, stream, tcp),
    {_, Ref} = S,
    Bools = [[socket:setopt(S, O, true), socket:getopt(S, O), socket:setopt(S, O, false),
              socket:getopt(S, O), socket:setopt(S, O, 1)]
             || O <- [{socket, broadcast}, {socket, keepalive}, {socket, reuseaddr},
                      {socket, reuseport}, {socket, oobinline}, {socket, dontroute},
                      {tcp, nodelay}]],
    Ints = [[socket:setopt(S, O, V), socket:getopt(S, O), socket:setopt(S, O, foo),
             socket:setopt(S, O, 1 bsl 40), socket:setopt(S, O, -1)]
            || {O, V} <- [{{tcp, keepcnt}, 5}, {{tcp, keepintvl}, 7}, {{ip, ttl}, 33},
                          {{socket, rcvbuf}, 8192}, {{socket, sndbuf}, 8192}]],
    {_, MRef} = spawn_monitor(fun() -> exit(socket:setopt(S, {otp, meta}, x)) end),
    NotOwner = receive {'DOWN', MRef, process, _, Reply} -> Reply end,
    Otp = [socket:getopt(S, {socket, type}), socket:getopt(S, {socket, acceptconn}),
           socket:setopt(S, {socket, type}, 1), socket:getopt(S, {otp, domain}),
           socket:getopt(S, {otp, use_registry}), socket:getopt(S, {otp, meta}),
           socket:setopt(S, {otp, meta}, {a, b}), socket:getopt(S, {otp, meta}), NotOwner,
           socket:setopt(S, {otp, use_registry}, false), socket:setopt(S, {otp, use_registry}, true),
           prim_socket:getopt(Ref, {socket, 12345}, 1),
           catch socket:setopt_native(S, {1 bsl 40, 1}, 1),
           catch socket:getopt_native(S, {6, 1 bsl 40}, integer),
           catch socket:setopt_native(S, {6, 1 bsl 40}, 1),
           catch socket:getopt_native(S, {1 bsl 40, 1}, integer),
           catch prim_socket:setopt(foo, {socket, broadcast}, true),
           catch prim_socket:getopt(foo, {socket, broadcast})],
    ok = socket:close(S),
    Closed = [socket:getopt(S, {socket, type}), socket:setopt(S, {socket, reuseaddr}, true),
              socket:getopt(S, {otp, domain}), socket:getopt(S, {otp, meta}),
              socket:setopt(S, {otp, meta}, x), socket:setopt(S, {otp, use_registry}, true),
              socket:getopt(S, {otp, use_registry})],
    {ok, D} = socket:open(inet, dgram, #{use_registry => false}),
    Registry = [lists:member(D, socket:which_sockets()), socket:getopt(D, {tcp, nodelay}),
                socket:getopt(D, {otp, use_registry}),
                socket:setopt(D, {otp, use_registry}, true), socket:getopt(D, {otp, use_registry}),
                lists:member(D, socket:which_sockets())],
    ok = socket:close(D),
    Bind = fun(Domain, Addr) ->
                   {ok, B} = socket:open(Domain, dgram),
                   R = socket:bind(B, Addr),
                   ok = socket:close(B),
                   R
           end,
    Binds = [Bind(inet, any), Bind(inet, loopback), Bind(inet, broadcast), Bind(inet6, any),
             Bind(inet6, loopback)],
    erlang:display({sockopt, Bools, Ints, [strip(E) || E <- Otp], Closed, Registry, Binds}),
    init:stop().

%% socket_select -kernel logger_level warning` runs socket_select/0. It
%% connects, listens, accepts, and cancels with socket:connect/2,3,
%% socket:listen/1,2, socket:accept/1,2, and socket:cancel/2, with
%% timeouts, nowait, and the select and abort messages. It replaces each
%% reference with an atom, and displays only the results that are the same
%% on Linux and macOS.
socket_select() ->
    process_flag(trap_exit, true),
    Parent = self(),
    {ok, L} = socket:open(inet, stream, tcp),
    {_, LRef} = L,
    ok = socket:bind(L, #{family => inet, addr => loopback, port => 0}),
    {ok, #{port := Port}} = socket:sockname(L),
    Addr = #{family => inet, addr => loopback, port => Port},
    Listen = [socket:listen(L), maps:get(rstates, socket:info(L)), socket:accept(L, 0)],
    {select, {select_info, accept, H1}} = socket:accept(L, nowait),
    Accepting = [maps:get(rstates, socket:info(L)), quiet(),
                 masked(socket:cancel(L, {select_info, accept, make_ref()}))],
    {ok, C} = socket:open(inet, stream, tcp),
    {_, CRef} = C,
    {select, {select_info, connect, CH}} = socket:connect(C, Addr, nowait),
    Connecting = maps:get(wstates, socket:info(C)),
    Other = fun(F) -> spawn(fun() -> Parent ! {other, catch F()} end),
                      receive {other, R} -> strip(R) end end,
    Others = [Other(fun() -> socket:connect(C, Addr) end),
              Other(fun() -> prim_socket:connect(CRef) end),
              masked(socket:cancel(C, {select_info, connect, make_ref()}))],
    Selected = [receive {'$socket', L, select, H1} -> accept_select after 5000 -> none end,
                receive {'$socket', C, select, CH} -> connect_select after 5000 -> none end],
    {ok, A} = socket:accept(L, H1),
    Connected = [socket:connect(C), strip(catch socket:connect(C)), socket:connect(C, Addr),
                 maps:get(wstates, socket:info(C)), maps:get(rstates, socket:info(L))],
    AInfo = maps:with([owner, rstates, wstates, domain, type, protocol], socket:info(A)),
    Accepted = [maps:get(owner, AInfo) =:= self(), maps:remove(owner, AInfo),
                lists:member(A, socket:which_sockets())],
    {select, {select_info, accept, H2}} = socket:accept(L, nowait),
    Cancel = [socket:cancel(L, {select_info, accept, H2}),
              masked(socket:cancel(L, {select_info, accept, H2})),
              maps:get(rstates, socket:info(L))],
    {select, {select_info, accept, H3}} = socket:accept(L, nowait),
    {select, {select_info, accept, H4}} = socket:accept(L, nowait),
    Again = [receive {'$socket', L, abort, {H3, Why}} -> Why after 0 -> none end,
             receive {'$socket', L, select, H3} -> stale after 0 -> none end],
    {ok, D} = socket:open(inet, stream, tcp),
    ok = socket:connect(D, Addr),
    Sent = [receive {'$socket', L, select, H4} -> accept_select after 5000 -> none end,
            socket:cancel(L, {select_info, accept, H4}), element(1, socket:accept(L, 0))],
    {ok, E} = socket:open(inet, stream, tcp),
    {select, {select_info, connect, EH}} = socket:connect(E, Addr, nowait),
    EConnect = [receive {'$socket', E, select, EH} -> connect_select after 5000 -> none end,
                socket:cancel(E, {select_info, connect, EH}), element(1, socket:accept(L, 5000))],
    {select, {select_info, accept, H5}} = socket:accept(L, nowait),
    ok = socket:close(E),
    ok = socket:close(L),
    Closed = [receive {'$socket', L, abort, {H5, Reason}} -> Reason after 0 -> none end,
              socket:accept(L, 0), socket:listen(L), socket:connect(L, Addr),
              socket:cancel(L, {select_info, accept, make_ref()}),
              socket:cancel(L, {select_info, connect, make_ref()}),
              prim_socket:cancel(LRef, foo, make_ref())],
    {ok, R} = socket:open(inet, stream, tcp),
    ok = socket:bind(R, #{family => inet, addr => loopback, port => 0}),
    {ok, #{port := RPort}} = socket:sockname(R),
    ok = socket:close(R),
    {ok, F} = socket:open(inet, stream, tcp),
    {_, FRef} = F,
    {ok, G} = socket:open(inet, dgram, udp),
    {_, GRef} = G,
    Errors = [socket:connect(F, Addr#{port := RPort}), socket:accept(F, 0),
              socket:listen(G),
              strip(catch prim_socket:connect(FRef)),
              strip(catch prim_socket:connect(FRef, foo, Addr)),
              strip(catch prim_socket:connect(FRef, make_ref(), [Addr])),
              prim_socket:listen(GRef, 1 bsl 40), prim_socket:listen(GRef, 1 bsl 70),
              strip(catch prim_socket:listen(GRef, foo)),
              strip(catch prim_socket:cancel(GRef, foo, make_ref())),
              strip(catch prim_socket:cancel(GRef, 1, make_ref())),
              strip(catch prim_socket:cancel(GRef, connect, foo))],
    Badargs = [strip(catch prim_socket:connect(make_ref())),
               strip(catch prim_socket:listen(make_ref(), 5)),
               strip(catch prim_socket:accept(make_ref(), make_ref())),
               strip(catch prim_socket:cancel(make_ref(), accept, make_ref()))],
    {ok, U} = socket:open(local, stream),
    Path = list_to_binary("/tmp/pbeam_select_probe_" ++ os:getpid()),
    _ = file:delete(Path),
    ok = socket:bind(U, #{family => local, path => Path}),
    ok = socket:listen(U),
    {ok, V} = socket:open(local, stream),
    {ok, W} = socket:open(local, stream),
    Local = [socket:connect(V, #{family => local, path => Path}),
             socket:connect(W, #{family => local, path => <<Path/binary, "x">>}),
             element(1, socket:accept(U))],
    _ = file:delete(Path),
    {ok, K} = socket:open(inet, stream, tcp),
    ok = socket:bind(K, #{family => inet, addr => loopback, port => 0}),
    ok = socket:listen(K),
    {ok, Addr2} = socket:sockname(K),
    {Acceptor, AMon} = spawn_monitor(fun() -> Parent ! {acceptor, socket:accept(K, nowait)},
                                              receive stop -> ok end end),
    receive {acceptor, {select, _}} -> ok end,
    Waiting = maps:get(rstates, socket:info(K)),
    exit(Acceptor, kill),
    receive {'DOWN', AMon, process, Acceptor, killed} -> ok end,
    Down = [maps:get(rstates, socket:info(K)), element(1, socket:accept(K, nowait))],
    {ok, C2} = socket:open(inet, stream, tcp),
    {Connector, CMon} = spawn_monitor(fun() -> Parent ! {connector, socket:connect(C2, Addr2, nowait)},
                                               receive stop -> ok end end),
    receive {connector, {select, _}} -> ok end,
    ConnectorWaiting = maps:get(wstates, socket:info(C2)),
    exit(Connector, kill),
    receive {'DOWN', CMon, process, Connector, killed} -> ok end,
    ConnectorDown = [ConnectorWaiting, maps:get(wstates, socket:info(C2))],
    {Owner, OMon} = spawn_monitor(fun() -> {ok, O} = socket:open(inet, stream, tcp),
                                           ok = socket:bind(O, #{family => inet, addr => loopback, port => 0}),
                                           ok = socket:listen(O),
                                           Parent ! {owner, O}, receive stop -> ok end end),
    O = receive {owner, Sock} -> Sock end,
    {select, {select_info, accept, OH}} = socket:accept(O, nowait),
    exit(Owner, kill),
    receive {'DOWN', OMon, process, Owner, killed} -> ok end,
    OwnerDown = [receive {'$socket', O, abort, {OH, OReason}} -> OReason after 1000 -> none end,
                 maps:get(rstates, socket:info(O))],
    erlang:display({socket_select, Listen, Accepting, Connecting, Others, Selected, Connected,
                    Accepted, Cancel, Again, Sent, EConnect, Closed, Errors, Badargs, Local, Waiting, Down,
                    ConnectorDown, OwnerDown}),
    init:stop().

%% `-s kernel_boot_probe socket_acceptors` calls socket:accept/2 from a
%% second process while the first process waits on the listen socket.
socket_acceptors() ->
    {ok, L} = socket:open(inet, stream, tcp),
    ok = socket:bind(L, #{family => inet, addr => loopback, port => 0}),
    ok = socket:listen(L),
    {select, _} = socket:accept(L, nowait),
    Parent = self(),
    spawn(fun() -> Parent ! socket:accept(L, nowait) end),
    receive Result -> erlang:display(Result) end,
    init:stop().

%% `-s kernel_boot_probe socket_connect_invalid` connects to an address
%% that the NIF does not decode.
socket_connect_invalid() ->
    {ok, S} = socket:open(inet, stream, tcp),
    erlang:display(socket:connect(S, #{family => inet, addr => {1, 2, 3}})),
    init:stop().

%% `-s kernel_boot_probe socket_recvfrom_flags` runs socket:recvfrom/4 with the
%% flag peek on inet and inet6 datagram sockets and an inet stream socket.
socket_recvfrom_flags() ->
    Loopback = #{family => inet, addr => {127, 0, 0, 1}, port => 0},
    {ok, U1} = socket:open(inet, dgram, udp),
    {ok, U2} = socket:open(inet, dgram, udp),
    ok = socket:bind(U1, Loopback),
    ok = socket:bind(U2, Loopback),
    {ok, A1} = socket:sockname(U1),
    {ok, A2} = socket:sockname(U2),
    From = fun({ok, {Addr, Data}}) -> {ok, {Addr =:= A1, Data}}; (Other) -> Other end,
    Udp = [socket:recvfrom(U2, 0, [peek], 0), socket:sendto(U1, <<"peek">>, A2),
           From(socket:recvfrom(U2, 0, [peek], 1000)), From(socket:recvfrom(U2, 2, [peek], 0)),
           From(socket:recvfrom(U2, 0, [], 0)), socket:recvfrom(U2, 0, [peek], 0)],
    Loopback6 = #{family => inet6, addr => loopback, port => 0},
    {ok, V1} = socket:open(inet6, dgram, udp),
    {ok, V2} = socket:open(inet6, dgram, udp),
    ok = socket:bind(V1, Loopback6),
    ok = socket:bind(V2, Loopback6),
    {ok, B1} = socket:sockname(V1),
    {ok, B2} = socket:sockname(V2),
    ok = socket:sendto(V1, <<"six">>, B2),
    Inet6 = [socket:recvfrom(V2, 0, [peek], 1000) =:= {ok, {B1, <<"six">>}}],
    {ok, L} = socket:open(inet, stream, tcp),
    ok = socket:bind(L, Loopback),
    ok = socket:listen(L),
    {ok, Addr} = socket:sockname(L),
    {ok, C} = socket:open(inet, stream, tcp),
    ok = socket:connect(C, Addr),
    {ok, A} = socket:accept(L),
    Stream = [socket:recvfrom(A, 0, [peek], 0), socket:send(C, <<"st">>), socket:recvfrom(A, 0, [peek], 1000),
              socket:recvfrom(A, 1, [peek], 0), socket:recvfrom(A, 0, [], 0), socket:shutdown(C, write),
              socket:recvfrom(A, 0, [peek], 1000)],
    Keys = [read_byte, read_pkg, read_pkg_max, read_tries, read_fails, read_waits],
    Counters = [maps:with(Keys, maps:get(counters, socket:info(S))) || S <- [U2, A]],
    erlang:display({socket_recvfrom_flags, Udp, Inet6, Stream, Counters}),
    init:stop().

%% `-s kernel_boot_probe socket_recvfrom_local_flags` receives with the flag
%% peek on a local datagram socket.
socket_recvfrom_local_flags() ->
    {ok, S} = socket:open(local, dgram, default),
    erlang:display(socket:recvfrom(S, 0, [peek], 0)),
    init:stop().

%% `-s kernel_boot_probe socket_recvfrom_errqueue` receives with the flag
%% errqueue on an inet datagram socket.
socket_recvfrom_errqueue() ->
    {ok, S} = socket:open(inet, dgram, udp),
    erlang:display({socket_recvfrom_errqueue, catch socket:recvfrom(S, 0, [errqueue], 0)}),
    init:stop().

%% `-s kernel_boot_probe socket_sendv` runs socket:sendv/2,3 on inet stream and
%% datagram sockets: an I/O vector of more than 1024 elements, a partial send,
%% nowait, the counters of socket:info/1, and the argument errors.
socket_sendv() ->
    Loopback = #{family => inet, addr => {127, 0, 0, 1}, port => 0},
    {ok, L} = socket:open(inet, stream, tcp),
    ok = socket:bind(L, Loopback),
    ok = socket:listen(L),
    {ok, Addr} = socket:sockname(L),
    {ok, C} = socket:open(inet, stream, tcp),
    {_, CRef} = C,
    ok = socket:connect(C, Addr),
    {ok, A} = socket:accept(L),
    Many = lists:duplicate(1500, <<"x">>),
    Stream = [socket:sendv(C, [<<"ab">>, <<>>, <<"cd">>]), socket:recv(A, 4, 1000), socket:sendv(C, []),
              socket:sendv(C, Many), socket:recv(A, 1500, 1000) =:= {ok, binary:copy(<<"x">>, 1500)},
              socket:sendv(C, lists:duplicate(1024, <<"y">>) ++ [<<>>]), socket:recv(A, 1024, 1000) =:= {ok, binary:copy(<<"y">>, 1024)},
              socket:sendv(C, [<<"z">>], 1000), socket:sendv(C, [<<"n">>], nowait), socket:recv(A, 2, 1000)],
    Big = binary:copy(<<"0123456789abcdef">>, 1 bsl 19),
    {select, {SI, Rest}} = socket:sendv(C, [Big, <<"tail">>], nowait),
    Partial = [element(1, SI), iolist_size(Rest) < byte_size(Big), lists:last(Rest), socket:cancel(C, SI)],
    flush_recv(A),
    Keys = [write_byte, write_pkg, write_pkg_max, write_tries, write_fails, write_waits],
    Counters = maps:with(Keys, maps:get(counters, socket:info(C))),
    {ok, U1} = socket:open(inet, dgram, udp),
    {ok, U2} = socket:open(inet, dgram, udp),
    {_, U1Ref} = U1,
    ok = socket:bind(U2, Loopback),
    {ok, A2} = socket:sockname(U2),
    NotConnected = socket:sendv(U1, [<<"a">>]),
    ok = socket:connect(U1, A2),
    Dgram = [NotConnected, socket:sendv(U1, [<<"da">>, <<"ta">>]), socket:recv(U2, 0, 1000),
             strip(catch socket:sendv(U1, Many)), strip(catch prim_socket:sendv(U1Ref, Many, make_ref()))],
    ok = socket:close(C),
    Errors = [socket:sendv(C, [<<"x">>]), strip(catch socket:sendv(U1, foo)), strip(catch socket:sendv(U1, [foo])),
              strip(catch prim_socket:sendv(make_ref(), [], make_ref())), strip(catch prim_socket:sendv(U1Ref, [], foo)),
              strip(catch prim_socket:sendv(U1Ref, foo, make_ref())), strip(catch prim_socket:sendv(U1Ref, [foo], make_ref())),
              strip(catch prim_socket:sendv(U1Ref, [<<"a">> | <<"b">>], make_ref())),
              strip(catch prim_socket:sendv(U1Ref, [<<1:3>>], make_ref())), strip(catch prim_socket:sendv(CRef, [], make_ref()))],
    erlang:display({socket_sendv, Stream, Partial, Counters, Dgram, Errors}),
    init:stop().

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% socket_io -kernel logger_level warning` runs socket_io/0. It sends and
%% receives on connected stream sockets with socket:send/2,3,4 and
%% socket:recv/1,2,3,4, with timeouts, the flag peek, nowait, a partial send, cancel, a
%% peer close, a close of a waiting reader, and the counters of
%% socket:info/1. It displays only the results that are the same on Linux
%% and macOS.
socket_io() ->
    process_flag(trap_exit, true),
    Parent = self(),
    {ok, L} = socket:open(inet, stream, tcp),
    ok = socket:bind(L, #{family => inet, addr => loopback, port => 0}),
    ok = socket:listen(L),
    {ok, Addr} = socket:sockname(L),
    {ok, C} = socket:open(inet, stream, tcp),
    {_, CRef} = C,
    ok = socket:connect(C, Addr),
    {ok, A} = socket:accept(L),
    Basic = [socket:send(C, <<"hello">>), socket:recv(A, 5),
             socket:send(C, [<<"ab">>, "cd"], []), socket:recv(A, 4, []),
             socket:send(C, <<"xyz">>, 1000), socket:recv(A, 0, 1000),
             socket:recv(A, 0, 0), socket:recv(A, 3, 0), socket:recv(A, 3, 50),
             socket:send(C, <<"pq">>), timer:sleep(50), socket:recv(A, 5, 0),
             socket:send(C, <<"rs">>), timer:sleep(50), socket:recv(A, 5, 50),
             socket:send(C, <<"tu">>, [], infinity), socket:recv(A, 2, [], infinity),
             socket:send(C, <<"pk">>), socket:recv(A, 2, [peek], 1000), socket:recv(A, 2, 1000)],
    {select, {select_info, recv, H1}} = socket:recv(A, 0, nowait),
    Selected = [maps:with([rstates, num_readers], socket:info(A)), socket:send(C, <<"late">>),
                receive {'$socket', A, select, H1} -> recv_select after 5000 -> none end,
                socket:recv(A, 0, H1)],
    {select, {select_info, recv, H2}} = socket:recv(A, 0, nowait),
    Cancel = [socket:cancel(A, {select_info, recv, H2}),
              masked(socket:cancel(A, {select_info, recv, H2}))],
    {select, {select_info, recv, H3}} = socket:recv(A, 0, nowait),
    ok = socket:send(C, <<"w">>),
    timer:sleep(50),
    Sent = [receive {'$socket', A, select, H3} -> recv_select after 5000 -> none end,
            socket:cancel(A, {select_info, recv, H3}), socket:recv(A, 0, 0)],
    Keys = [read_byte, read_pkg, read_pkg_max, read_fails, write_byte, write_pkg, write_pkg_max, write_fails],
    Counters = [maps:with(Keys, maps:get(counters, socket:info(S))) || S <- [A, C]],
    Big = binary:copy(<<"0123456789abcdef">>, 1 bsl 19),
    Size = byte_size(Big),
    {select, {SI1, Rest}} = socket:send(C, Big, nowait),
    Partial = [element(1, SI1), byte_size(Rest) < Size, maps:with([wstates, num_writers], socket:info(C))],
    spawn(fun() -> Parent ! {got, socket:recv(A, Size)} end),
    Continued = socket:send(C, Rest, SI1, infinity),
    Written = [Continued, receive {got, {ok, Got}} -> Got =:= Big after 5000 -> none end],
    {select, {SI2, _}} = socket:send(C, Big, nowait),
    Timeout = [socket:cancel(C, SI2), masked(socket:cancel(C, SI2)),
               element(1, socket:send(C, Big, 50))],
    {ok, _} = socket:recv(A, 0, 1000),
    flush_recv(A),
    Spawned = spawn(fun() -> Parent ! {reader, socket:recv(A, 0)} end),
    timer:sleep(50),
    ok = socket:close(A),
    Closed = [receive {reader, R} -> R after 5000 -> none end, socket:send(A, <<"x">>), socket:recv(A, 0),
              socket:cancel(A, {select_info, recv, make_ref()}),
              socket:cancel(A, {select_info, send, make_ref()}), is_pid(Spawned)],
    {ok, C2} = socket:open(inet, stream, tcp),
    ok = socket:connect(C2, Addr),
    {ok, A2} = socket:accept(L),
    PeerClosed = [socket:recv(C2, 0, 0), socket:close(A2), socket:recv(C2, 0), socket:recv(C2, 0, nowait),
                  maps:get(read_fails, maps:get(counters, socket:info(C2)))],
    {ok, D} = socket:open(inet, stream, tcp),
    {_, DRef} = D,
    {ok, L2} = socket:open(inet, stream, tcp),
    ok = socket:bind(L2, #{family => inet, addr => loopback, port => 0}),
    ok = socket:listen(L2),
    {select, _} = socket:accept(L2, nowait),
    Errors = [socket:recv(D, 0, 0), socket:recv(L2, 0, 0),
              strip(catch prim_socket:send(DRef, foo, 0, make_ref())),
              strip(catch prim_socket:send(DRef, <<1:3>>, 0, make_ref())),
              strip(catch prim_socket:send(DRef, <<>>, 0, foo)),
              strip(catch prim_socket:send(DRef, <<>>, foo, make_ref())),
              strip(catch prim_socket:send(DRef, <<>>, 1 bsl 40, make_ref())),
              strip(catch prim_socket:send(make_ref(), <<>>, 0, make_ref())),
              strip(catch prim_socket:recv(make_ref(), 0, [], make_ref())),
              strip(catch prim_socket:recv(DRef, 0, [], foo)),
              strip(catch prim_socket:recv(DRef, foo, [], make_ref())),
              strip(catch prim_socket:recv(DRef, -1, [], make_ref())),
              strip(catch prim_socket:recv(DRef, 1 bsl 70, [], make_ref())),
              strip(catch prim_socket:recv(DRef, 0, [], zero))],
    _ = CRef,
    erlang:display({socket_io, Basic, Selected, Cancel, Sent, Partial, Written, Timeout,
                    Counters, Closed, PeerClosed, Errors}),
    init:stop().

%% `erl -noshell -boot start_clean -pa fixtures/boot -s kernel_boot_probe
%% socket_dgram -kernel logger_level warning` runs socket_dgram/0. It runs
%% socket:sendto/3,4,5, socket:recvfrom/1,2,3,4, socket:peername/1, and
%% socket:shutdown/2 on inet and inet6 datagram, local datagram, and inet stream
%% sockets, with timeouts, nowait, cancel, a partial send, the counters of
%% socket:info/1, and the argument errors. The errno of a shutdown after a
%% peer shutdown depends on the host, so compare it on one host only.
socket_dgram() ->
    Loopback = #{family => inet, addr => loopback, port => 0},
    {ok, U1} = socket:open(inet, dgram, udp),
    {ok, U2} = socket:open(inet, dgram, udp),
    {_, U1Ref} = U1,
    ok = socket:bind(U1, Loopback),
    ok = socket:bind(U2, Loopback),
    {ok, A1} = socket:sockname(U1),
    {ok, A2} = socket:sockname(U2),
    From = fun({ok, {Addr, Data}}) -> {ok, {Addr =:= A1, Data}}; (Other) -> Other end,
    Udp = [socket:peername(U1), socket:sendto(U1, <<"ping">>, A2), From(socket:recvfrom(U2)),
           socket:sendto(U1, [<<"ab">>, "cd"], A2, []), From(socket:recvfrom(U2, 2, 1000)), socket:recvfrom(U2, 0, 0), socket:recvfrom(U2, 0, 50),
           socket:sendto(U1, <<"xyz">>, A2, 1000), From(socket:recvfrom(U2, [], 1000)),
           socket:sendto(U1, <<"tu">>, A2, [], infinity), From(socket:recvfrom(U2, 0, [], infinity)),
           socket:sendto(U1, <<>>, A2), From(socket:recvfrom(U2, 0, 1000)),
           socket:sendto(U1, <<"r">>, A2), timer:sleep(50), From(socket:recvfrom(U2, 0, 0))],
    {select, {select_info, recvfrom, H1}} = socket:recvfrom(U2, 0, nowait),
    Selected = [maps:with([rstates, num_readers], socket:info(U2)), socket:sendto(U1, <<"late">>, A2),
                receive {'$socket', U2, select, H1} -> recv_select after 5000 -> none end,
                From(socket:recvfrom(U2, 0, [], H1))],
    {select, {select_info, recvfrom, H2}} = socket:recvfrom(U2, 0, nowait),
    Cancel = [socket:cancel(U2, {select_info, recvfrom, H2}),
              masked(socket:cancel(U2, {select_info, recvfrom, H2})),
              masked(socket:cancel(U2, {select_info, sendto, make_ref()}))],
    ok = socket:connect(U1, A2),
    Connected = [socket:peername(U1) =:= {ok, A2}, socket:send(U1, <<"c">>), From(socket:recvfrom(U2, 0, 1000)),
                 socket:send(U1, <<"d">>), socket:recv(U2, 0, 1000)],
    Keys = [read_byte, read_pkg, read_pkg_max, read_fails, write_byte, write_pkg, write_pkg_max, write_fails],
    Counters = [maps:with(Keys, maps:get(counters, socket:info(S))) || S <- [U1, U2]],
    Loopback6 = #{family => inet6, addr => loopback, port => 0},
    {ok, V1} = socket:open(inet6, dgram, udp),
    {ok, V2} = socket:open(inet6, dgram, udp),
    ok = socket:bind(V1, Loopback6),
    ok = socket:bind(V2, Loopback6),
    {ok, B1} = socket:sockname(V1),
    {ok, B2} = socket:sockname(V2),
    ok = socket:sendto(V1, <<"six">>, B2),
    Received6 = socket:recvfrom(V2, 0, 1000),
    ok = socket:connect(V1, B2),
    Inet6 = [Received6 =:= {ok, {B1, <<"six">>}}, element(1, Received6), socket:peername(V1) =:= {ok, B2}],
    _ = [socket:close(S) || S <- [V1, V2]],
    Path1 = <<"/tmp/pbeam_socket_dgram_1">>,
    Path2 = <<"/tmp/pbeam_socket_dgram_2">>,
    _ = [file:delete(P) || P <- [Path1, Path2]],
    {ok, L1} = socket:open(local, dgram, default),
    {ok, L2} = socket:open(local, dgram, default),
    {ok, L3} = socket:open(local, dgram, default),
    ok = socket:bind(L1, #{family => local, path => Path1}),
    ok = socket:bind(L2, #{family => local, path => Path2}),
    Local = [socket:sendto(L1, <<"loc">>, #{family => local, path => Path2}), socket:recvfrom(L2, 0, 1000),
             socket:sendto(L3, <<"anon">>, #{family => local, path => Path2}), socket:recvfrom(L2, 0, 1000),
             socket:peername(L2)],
    _ = [socket:close(S) || S <- [L1, L2, L3]],
    _ = [file:delete(P) || P <- [Path1, Path2]],
    {ok, L} = socket:open(inet, stream, tcp),
    ok = socket:bind(L, Loopback),
    ok = socket:listen(L),
    {ok, Addr} = socket:sockname(L),
    {ok, C} = socket:open(inet, stream, tcp),
    {_, CRef} = C,
    NotConnected = socket:peername(C),
    ok = socket:connect(C, Addr),
    {ok, A} = socket:accept(L),
    {select, {select_info, recvfrom, H3}} = socket:recvfrom(A, 0, nowait),
    Stream = [NotConnected, socket:peername(C) =:= {ok, Addr}, socket:sendto(C, <<"st">>, Addr),
              receive {'$socket', A, select, H3} -> recv_select after 5000 -> none end,
              socket:recvfrom(A, 0, [], H3), socket:recvfrom(A, 0, 0)],
    Big = binary:copy(<<"0123456789abcdef">>, 1 bsl 19),
    {select, {SI1, Rest}} = socket:sendto(C, Big, Addr, nowait),
    Partial = [element(1, SI1), byte_size(Rest) < byte_size(Big), socket:cancel(C, SI1)],
    flush_recv(A),
    {ok, D} = socket:open(inet, stream, tcp),
    Shutdown = [socket:shutdown(C, write), socket:recvfrom(A, 0, 1000), socket:send(A, <<"back">>),
                socket:recv(C, 0, 1000), socket:shutdown(A, read_write), socket:shutdown(A, read),
                socket:shutdown(D, write), maps:get(read_fails, maps:get(counters, socket:info(A)))],
    ok = socket:close(U2),
    Closed = [socket:recvfrom(U2, 0, 0), socket:sendto(U2, <<"x">>, A1), socket:peername(U2),
              socket:shutdown(U2, read)],
    {_, U2Ref} = U2,
    Errors = [strip(catch prim_socket:shutdown(CRef, foo)), strip(catch prim_socket:shutdown(make_ref(), read)),
              strip(catch prim_socket:shutdown(U2Ref, foo)), strip(catch prim_socket:peername(make_ref())),
              strip(catch prim_socket:sendto(U1Ref, foo, A2, [], make_ref())),
              strip(catch prim_socket:sendto(U1Ref, <<>>, A2, [], foo)),
              strip(catch prim_socket:sendto(make_ref(), <<>>, A2, [], make_ref())),
              strip(catch prim_socket:sendto(U2Ref, <<>>, #{family => inet, addr => {1, 2, 3}, port => 1}, [], make_ref())),
              strip(catch prim_socket:sendto(U1Ref, <<>>, {A2, A2, 1 bsl 40}, make_ref())),
              strip(catch prim_socket:sendto(U1Ref, <<>>, {A2, A2, foo}, make_ref())),
              strip(catch prim_socket:sendto(U1Ref, <<>>, {bad, #{family => inet, addr => {1, 2, 3}, port => 1}, 0},
                                             make_ref())),
              strip(catch prim_socket:recvfrom(make_ref(), 0, [], make_ref())),
              strip(catch prim_socket:recvfrom(U1Ref, 0, [], foo)),
              strip(catch prim_socket:recvfrom(U1Ref, foo, [], make_ref())),
              strip(catch prim_socket:recvfrom(U1Ref, -1, [], make_ref())),
              strip(catch prim_socket:recvfrom(U1Ref, 1 bsl 70, [], make_ref())),
              strip(catch prim_socket:recvfrom(U2Ref, 0, [], zero))],
    erlang:display({socket_dgram, Udp, Selected, Cancel, Connected, Counters, Inet6, Local, Stream, Partial,
                    Shutdown, Closed, Errors}),
    init:stop().

%% `-s kernel_boot_probe socket_backend_tcp` sends and receives with gen_tcp
%% on the socket backend (gen_tcp_socket.erl).
socket_backend_tcp() ->
    Opts = [{inet_backend, socket}, binary, {active, false}],
    {ok, L} = gen_tcp:listen(0, Opts ++ [{ip, {127, 0, 0, 1}}]),
    {ok, Port} = inet:port(L),
    {ok, C} = gen_tcp:connect({127, 0, 0, 1}, Port, Opts),
    {ok, A} = gen_tcp:accept(L, 1000),
    Result = [gen_tcp:send(C, <<"ping">>), gen_tcp:recv(A, 4, 1000), gen_tcp:send(A, [<<"po">>, "ng"]),
              gen_tcp:recv(C, 0, 1000), gen_tcp:recv(C, 0, 0), gen_tcp:shutdown(C, write), gen_tcp:recv(A, 0, 1000),
              gen_tcp:close(C), gen_tcp:close(A), gen_tcp:close(L)],
    erlang:display({socket_backend_tcp, Result}),
    init:stop().

%% `-s kernel_boot_probe socket_backend_udp` sends and receives with gen_udp
%% on the socket backend (gen_udp_socket.erl).
socket_backend_udp() ->
    Opts = [{inet_backend, socket}, binary, {active, false}, {ip, {127, 0, 0, 1}}],
    {ok, U1} = gen_udp:open(0, Opts),
    {ok, U2} = gen_udp:open(0, Opts),
    {ok, P1} = inet:port(U1),
    {ok, P2} = inet:port(U2),
    Result = [gen_udp:send(U1, {127, 0, 0, 1}, P2, <<"ping">>), gen_udp:recv(U2, 0, 1000) =:= {ok, {{127, 0, 0, 1}, P1, <<"ping">>}},
              gen_udp:send(U2, {127, 0, 0, 1}, P1, [<<"po">>, "ng"]), element(3, element(2, gen_udp:recv(U1, 0, 1000))),
              gen_udp:recv(U1, 0, 0), gen_udp:close(U1), gen_udp:close(U2)],
    erlang:display({socket_backend_udp, Result}),
    init:stop().

%% `-s kernel_boot_probe sockopt_host` runs socket:setopt/3 and
%% socket:getopt/2 with the host options of the levels socket, ip, ipv6,
%% and tcp that have a boolean, an integer, a hop count, an address, a
%% membership, a TOS, a string, a domain, or a protocol value: the values,
%% the invalid values, and the options that the host does not support.
%% PBeam has no handler for {ip, recvttl} and {socket, bindtodevice} on
%% macOS, so the probe sets them only on Linux.
sockopt_host() ->
    process_flag(trap_exit, true),
    {ok, T} = socket:open(inet, stream, tcp),
    {ok, U} = socket:open(inet, dgram, udp),
    {ok, U6} = socket:open(inet6, dgram, udp),
    Linux = os:type() =:= {unix, linux},
    Bools = [[socket:setopt(S, O, true), socket:getopt(S, O), socket:setopt(S, O, false),
              socket:getopt(S, O), socket:setopt(S, O, 1)]
             || {S, O} <- [{U, {socket, timestamp}}, {U, {ip, multicast_loop}}, {U, {ip, recvtos}},
                           {U, {ip, pktinfo}}, {U, {ip, recvif}}, {U, {ip, recvdstaddr}},
                           {U, {ip, hdrincl}}, {U6, {ipv6, recvtclass}}, {U6, {ipv6, multicast_loop}},
                           {T, {tcp, cork}}, {U, {socket, passcred}}, {U, {socket, timestampns}},
                           {U, {ip, freebind}}, {U, {ip, transparent}}, {U, {ip, recverr}},
                           {U, {ip, multicast_all}}, {U, {ip, recvorigdstaddr}}, {U6, {ipv6, recverr}},
                           {U6, {ipv6, recvhoplimit}}, {U6, {ipv6, recvpktinfo}}]
                          ++ [{U, {ip, recvttl}} || Linux]],
    Ints = [[socket:setopt(S, O, V), socket:getopt(S, O), socket:setopt(S, O, foo),
             socket:setopt(S, O, -1), socket:getopt(S, O)]
            || {S, O, V} <- [{U, {ip, multicast_ttl}, 5}, {U6, {ipv6, tclass}, 16},
                             {U6, {ipv6, multicast_if}, 0}, {T, {tcp, maxseg}, 1200},
                             {T, {tcp, keepidle}, 30}, {T, {tcp, user_timeout}, 1000}]],
    Hops = [[socket:setopt(U6, O, 7), socket:getopt(U6, O), socket:setopt(U6, O, default),
             socket:getopt(U6, O), socket:setopt(U6, O, 256), socket:setopt(U6, O, -1),
             socket:setopt(U6, O, 1 bsl 40), socket:setopt(U6, O, foo)]
            || O <- [{ipv6, multicast_hops}, {ipv6, unicast_hops}]],
    UserTimeout = [socket:setopt(T, {tcp, user_timeout}, 1 bsl 31),
                   socket:setopt(T, {tcp, user_timeout}, 1 bsl 32)],
    If = {ip, multicast_if},
    Ifs = [socket:setopt(U, If, {127, 0, 0, 1}), socket:getopt(U, If), socket:setopt(U, If, any),
           socket:getopt(U, If), socket:setopt(U, If, foo), socket:setopt(U, If, {1, 2, 3})],
    M = #{multiaddr => {224, 0, 0, 251}, interface => {127, 0, 0, 1}},
    Add = {ip, add_membership},
    Drop = {ip, drop_membership},
    Members = [socket:setopt(U, Add, M), socket:getopt(U, Add), socket:setopt(U, Drop, M),
               socket:getopt(U, Drop), socket:setopt(U, Add, #{multiaddr => {224, 0, 0, 251}}),
               socket:setopt(U, Add, M#{interface := foo}), socket:setopt(U, Add, foo),
               socket:setopt(U, Drop, M#{interface := any})],
    Tos = [[socket:setopt(U, {ip, tos}, V), socket:getopt(U, {ip, tos})]
           || V <- [lowdelay, throughput, reliability, mincost, cs0, cs1, cs2, cs3, cs4, cs5, cs6, cs7,
                    af11, af12, af13, af21, af22, af23, af31, af32, af33, af41, af42, af43, ef,
                    voice_admit, le, nqb, 16#b8, 16#e0, 16#ff, 0, #{precedence => 5, tos => lowdelay},
                    #{precedence => flash, tos => default}, #{precedence => routine, tos => 3},
                    #{precedence => 300, tos => 260, extra => 1}]]
        ++ [socket:setopt(U, {ip, tos}, V)
            || V <- [default, foo, 1 bsl 40, 1.5, "x", #{tos => lowdelay}, #{precedence => 1},
                     #{precedence => -1, tos => 0}, #{precedence => 1 bsl 33, tos => 0},
                     #{precedence => 1, tos => foo}, #{precedence => foo, tos => 0},
                     #{precedence => 1.0, tos => 0}, #{precedence => 1, tos => "x"}]]
        ++ [socket:setopt(T, {ip, tos}, -1), socket:setopt(T, {ip, tos}, 16#1ff)],
    {ok, C} = socket:open(inet, dgram, udp),
    ok = socket:connect(C, #{family => inet, addr => loopback, port => 9}),
    Mtu = [socket:setopt(U, {ip, mtu}, 1500), socket:getopt(U, {ip, mtu}), socket:getopt(C, {ip, mtu})],
    B = {socket, bindtodevice},
    Device = [[socket:getopt(S, B), socket:setopt(S, B, "lo"), socket:getopt(S, B), socket:setopt(S, B, <<"lo">>),
               socket:setopt(S, B, ""), socket:getopt(S, B), socket:setopt(S, B, <<"lo", 0, "x">>), socket:getopt(S, B),
               socket:setopt(S, B, "abcdefghijklmno"), socket:setopt(S, B, "abcdefghijklmnop"),
               socket:setopt(S, B, <<"abcdefghijklmnopqrstu">>), socket:setopt(S, B, [108, 256]),
               socket:setopt(S, B, [108 | 111]), socket:setopt(S, B, <<1:3>>), socket:setopt(S, B, foo)]
              || S <- [C], Linux],
    {ok, L} = socket:open(local, stream, default),
    Kinds = [socket:getopt(S, {socket, O}) || S <- [T, U6, L], O <- [domain, protocol]]
        ++ [socket:setopt(T, {socket, domain}, inet), socket:setopt(T, {socket, protocol}, tcp)],
    Cc = {tcp, congestion},
    Congestion = [socket:getopt(T, Cc), socket:setopt(T, Cc, "reno"), socket:getopt(T, Cc),
                  socket:setopt(T, Cc, <<"cubic", 0, "x">>), socket:getopt(T, Cc),
                  socket:setopt(T, Cc, "abcdefghijklmnop"), socket:setopt(T, Cc, "abcdefghijklmnopq"),
                  socket:setopt(T, Cc, <<"abcdefghijklmnopqrstu">>), socket:setopt(T, Cc, foo)],
    M6 = #{multiaddr => {16#ff02, 0, 0, 0, 0, 0, 0, 16#fb}, interface => 0},
    Add6 = {ipv6, add_membership},
    Drop6 = {ipv6, drop_membership},
    Members6 = [socket:setopt(U6, Add6, M6), socket:getopt(U6, Add6), socket:setopt(U6, Drop6, M6),
                socket:getopt(U6, Drop6), socket:setopt(U6, Add6, M6#{interface := -1}),
                socket:setopt(U6, Add6, M6#{interface := 1 bsl 32}), socket:setopt(U6, Add6, M6#{multiaddr := foo}),
                socket:setopt(U6, Add6, #{interface => 0}), socket:setopt(U6, Drop6, foo)],
    [ok = socket:close(S) || S <- [T, U, U6, C, L]],
    erlang:display(unmap({sockopt_host, Bools, Ints, UserTimeout, Hops, Ifs, Members, Tos, Mtu, Device, Kinds,
                          Congestion, Members6})),
    init:stop().

%% `-s kernel_boot_probe sockopt_otp` runs socket:setopt/3 and
%% socket:getopt/2 with the options of the level otp and with
%% {socket, linger}: the values, the invalid values, the inheritance of an
%% accepted socket, the receive buffer of a receive with length 0, a new
%% controlling process, and a closed socket.
sockopt_otp() ->
    process_flag(trap_exit, true),
    {ok, L} = socket:open(inet, stream, tcp),
    Flags = [[socket:getopt(L, {otp, O}), socket:setopt(L, {otp, O}, true), socket:getopt(L, {otp, O}),
              socket:setopt(L, {otp, O}, false), socket:setopt(L, {otp, O}, 1)]
             || O <- [iow]]
        ++ [[socket:getopt(L, {otp, O}), socket:setopt(L, {otp, O}, false), socket:setopt(L, {otp, O}, 1)]
            || O <- [debug, select_read]],
    Sizes = [[socket:getopt(L, {otp, O}), socket:setopt(L, {otp, O}, 100), socket:getopt(L, {otp, O}),
              socket:setopt(L, {otp, O}, default), socket:getopt(L, {otp, O}), socket:setopt(L, {otp, O}, 0),
              socket:setopt(L, {otp, O}, -1), socket:setopt(L, {otp, O}, foo),
              socket:setopt(L, {otp, O}, {0, 50}), socket:setopt(L, {otp, O}, 1 bsl 64),
              socket:setopt(L, {otp, O}, 1 bsl 63), socket:getopt(L, {otp, O}), socket:setopt(L, {otp, O}, default)]
             || O <- [rcvbuf, rcvctrlbuf, sndctrlbuf]],
    Linger = [socket:getopt(L, {socket, linger}), socket:setopt(L, {socket, linger}, #{onoff => true, linger => 7}),
              socket:getopt(L, {socket, linger}), socket:setopt(L, {socket, linger}, abort),
              socket:getopt(L, {socket, linger}), socket:setopt(L, {socket, linger}, #{onoff => false, linger => 0}),
              socket:setopt(L, {socket, linger}, #{onoff => true, linger => -1}),
              socket:setopt(L, {socket, linger}, #{onoff => 1, linger => 1}),
              socket:setopt(L, {socket, linger}, #{onoff => true}), socket:setopt(L, {socket, linger}, foo)],
    %% {socket, priority} is a Linux option; the set of linger keeps it.
    Priority = [socket:setopt(L, {socket, priority}, 3), socket:setopt(L, {socket, linger}, abort),
                socket:getopt(L, {socket, priority})],
    Raise = [strip(catch socket:setopt(L, {otp, fd}, 1)), strip(catch socket:setopt(L, {otp, domain}, inet)),
             is_integer(element(2, socket:getopt(L, {otp, fd})))],
    ok = socket:setopt(L, {otp, iow}, true),
    ok = socket:setopt(L, {otp, rcvbuf}, 4),
    ok = socket:setopt(L, {otp, rcvctrlbuf}, 200),
    ok = socket:setopt(L, {otp, sndctrlbuf}, 300),
    ok = socket:bind(L, #{family => inet, addr => loopback, port => 0}),
    ok = socket:listen(L),
    {ok, #{port := Port}} = socket:sockname(L),
    {ok, C} = socket:open(inet, stream, tcp),
    ok = socket:connect(C, #{family => inet, addr => loopback, port => Port}),
    {ok, A} = socket:accept(L, 1000),
    Inherit = [socket:getopt(A, {otp, O}) || O <- [iow, rcvbuf, rcvctrlbuf, sndctrlbuf]],
    ok = socket:send(C, <<"0123456789">>),
    timer:sleep(100),
    Recv = [socket:recv(A, 0, 1000), socket:recv(A, 0, 1000), socket:recv(A, 0, 1000)],
    Self = self(),
    P = spawn(fun() -> receive stop -> ok end end),
    MRef = erlang:monitor(process, P),
    Owner = [socket:getopt(A, {otp, controlling_process}) =:= {ok, Self},
             socket:setopt(A, {otp, controlling_process}, foo), socket:setopt(A, {otp, controlling_process}, P),
             socket:getopt(A, {otp, controlling_process}) =:= {ok, P},
             socket:setopt(A, {otp, controlling_process}, Self), socket:setopt(A, {otp, meta}, x)],
    P ! stop,
    receive {'DOWN', MRef, process, P, _} -> ok end,
    timer:sleep(100),
    Down = socket:getopt(A, {otp, iow}),
    ok = socket:close(C),
    ok = socket:close(L),
    Closed = [socket:getopt(L, {otp, O}) || O <- [iow, debug, select_read, rcvbuf, rcvctrlbuf, sndctrlbuf, fd,
                                               controlling_process]]
        ++ [socket:setopt(L, {otp, O}, V) || {O, V} <- [{iow, true}, {debug, false}, {select_read, false},
                                                        {rcvbuf, 1}, {rcvctrlbuf, 1}, {sndctrlbuf, 1},
                                                        {controlling_process, Self}]],
    erlang:display(unmap({sockopt_otp, Flags, Sizes, Linger, Priority, Raise, Inherit, Recv, Owner, Down, Closed})),
    init:stop().

%% unmap/1 replaces each map with its sorted list of pairs.
unmap(M) when is_map(M) -> {map, lists:sort(unmap(maps:to_list(M)))};
unmap(T) when is_tuple(T) -> list_to_tuple(unmap(tuple_to_list(T)));
unmap(L) when is_list(L) -> [unmap(E) || E <- L];
unmap(Other) -> Other.

flush_recv(S) ->
    case socket:recv(S, 0, 100) of
        {ok, _} -> flush_recv(S);
        _ -> ok
    end.

quiet() ->
    receive Message -> Message after 50 -> quiet end.

masked({error, {invalid, {select_info, Tag, Ref}}}) when is_reference(Ref) ->
    {error, {invalid, {select_info, Tag, ref}}};
masked(Other) -> Other.

native_socket({ok, S}) ->
    Info = maps:with([domain, type, protocol], socket:info(S)),
    ok = socket:close(S),
    Info;
native_socket(Error) -> Error.

strip({'EXIT', {Reason, _}}) -> {'EXIT', Reason};
strip(Other) -> Other.

report() ->
    Controller = whereis(application_controller),
    Kernel = application:which_applications(),
    StartKernel = application:start(kernel),
    StartMissing = application:start(kernel_boot_missing),
    EnsureKernel = application:ensure_all_started(kernel),
    EnsureDep = application:ensure_all_started(kernel_boot_dep),
    Running = application:which_applications(),
    StartAgain = application:start(kernel_boot_app),
    StopDep = application:stop(kernel_boot_dep),
    StopDepAgain = application:stop(kernel_boot_dep),
    EnsureTemporary = application:ensure_all_started([kernel_boot_dep], temporary),
    EnsureMissing = application:ensure_all_started(kernel_boot_missing),
    StopBadArg = try init:stop(stop) catch Class:Reason -> {Class, Reason} end,
    %% init kills each process that is not a system process at the stop
    %% (init.erl:1082-1093); init is a system process (erl_init.c:355-399).
    System = [erts_internal:is_system_process(Pid) || Pid <- [whereis(init), Controller, self()]],
    SystemBadArg = try erts_internal:is_system_process(init) catch Class2:Reason2 -> {Class2, Reason2} end,
    erlang:display({controller, process_info(Controller, registered_name),
                    controller_linked_to_init,
                    lists:member(Controller, element(2, process_info(whereis(init), links))),
                    kernel, Kernel,
                    start_kernel, StartKernel,
                    start_missing, StartMissing,
                    ensure_kernel, EnsureKernel,
                    ensure_dep, EnsureDep,
                    running, Running,
                    start_again, StartAgain,
                    stop_dep, StopDep,
                    stop_dep_again, StopDepAgain,
                    ensure_temporary, EnsureTemporary,
                    ensure_missing, EnsureMissing,
                    stop_badarg, StopBadArg,
                    system, System,
                    system_badarg, SystemBadArg,
                    status, init:get_status()}).

start(normal, []) ->
    supervisor:start_link({local, kernel_boot_sup}, ?MODULE, []).

stop(State) ->
    erlang:display({stop, kernel_boot_app, State, whereis(kernel_boot_sup),
                    init:get_status()}),
    ok.

init([]) ->
    {ok, {#{}, []}}.
