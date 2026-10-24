-module(net_probe).
-export([main/0, text/0, debug_on/0, ifaddrs_text/0, ifaddrs_debug/0, ifaddrs_netns/0]).

%% The cases of `nif_info/0`, `nif_command/1`, and `nif_gethostname/0` of
%% the `net` NIF library (`prim_net_nif.c:848-1021`). `main/0` gives one
%% pair for each case: the result of the call, or the class and the reason
%% of its exception.
main() ->
    [{info, call(fun() -> lists:sort(maps:to_list(prim_net:info())) end)},
     {debug_off, call(fun() -> prim_net:debug(false) end)},
     {info_debug, call(fun() -> maps:get(debug, prim_net:info()) end)},
     {debug_bad, call(fun() -> prim_net:debug(1) end)},
     {hostname, call(fun() -> element(1, net:gethostname()) end)}
     | lookups() ++ interfaces()].

%% The cases of `nif_if_name2index/1`, `nif_if_index2name/1`,
%% `nif_if_names/0`, and the four Windows functions
%% (`prim_net_nif.c:1914-1950`, `3124-3800`, `4272-4494`). ERTS and PBeam
%% read the interfaces of one host.
interfaces() ->
    {ok, [{Index, Name} | _]} = net:if_names(),
    [{Name1, call(Fun)} || {Name1, Fun} <-
        [{if_names, fun() -> net:if_names() end},
         {if_name, fun() -> net:if_name2index(Name) end},
         {if_index, fun() -> net:if_index2name(Index) end},
         {if_name_nul, fun() -> net:if_name2index(Name ++ [0, $x]) end},
         {if_name_none, fun() -> net:if_name2index("pbeam-no-if") end},
         {if_name_16, fun() -> net:if_name2index("1234567890123456") end},
         {if_name_17, fun() -> net:if_name2index("12345678901234567") end},
         {if_name_wide, fun() -> net:if_name2index([256]) end},
         {if_name_atom, fun() -> net:if_name2index([a]) end},
         {if_index_none, fun() -> net:if_index2name(16#7fffffff) end},
         {if_index_negative, fun() -> net:if_index2name(-1) end},
         {if_index_big, fun() -> net:if_index2name(1 bsl 32) end},
         {adapters, fun() -> prim_net:get_adapters_addresses(#{}) end},
         {if_entry, fun() -> prim_net:get_if_entry(#{}) end},
         {interface_info, fun() -> prim_net:get_interface_info(#{}) end},
         {ip_address_table, fun() -> prim_net:get_ip_address_table(#{}) end}]].

%% The cases of `nif_getifaddrs/1` (`prim_net_nif.c:1381-1877`), as
%% `text/0` writes them. The pinned ERTS on Linux defines `HAVE_SETNS`, so
%% a `netns` that is not a string raises `badarg` there.
ifaddrs_text() ->
    Cases = [call(fun() -> net:getifaddrs() end),
             call(fun() -> prim_net:getifaddrs(#{debug => false}) end),
             call(fun() -> prim_net:getifaddrs(#{debug => 1}) end),
             call(fun() -> prim_net:getifaddrs(#{netns => 1}) end)],
    iolist_to_binary([[binary:encode_hex(term_to_binary(canonical(unread(Case)))), $\n]
                      || Case <- Cases]).

%% Give zero for the address of an `inet` `dstaddr` of an `inet6` entry:
%% the `AF_UNSPEC` `ifa_dstaddr` of a macOS `utun` interface, of which ERTS
%% reads the bytes past `sa_len` and PBeam gives zero (row "NIF stubs" of
%% `docs/erts-differences.md`).
unread({value, {ok, Entries}}) ->
    {value, {ok, [unread_entry(Entry) || Entry <- Entries]}};
unread(Case) ->
    Case.

unread_entry(#{addr := #{family := inet6}, dstaddr := #{family := inet} = Dst} = Entry) ->
    Entry#{dstaddr := Dst#{addr := {0, 0, 0, 0}}};
unread_entry(Entry) ->
    Entry.

%% Ask `nif_getifaddrs/1` for its debug trace.
ifaddrs_debug() ->
    prim_net:getifaddrs(#{debug => true}).

%% Ask `nif_getifaddrs/1` for the interfaces of a network namespace.
ifaddrs_netns() ->
    prim_net:getifaddrs(#{netns => "/var/run/netns/pbeam"}).

%% The cases of `nif_getnameinfo/2`, `nif_getaddrinfo/3`,
%% `nif_getservbyname/2`, and `nif_getservbyport/2`
%% (`prim_net_nif.c:1036-1362`, `4089-4257`). Each lookup uses only the
%% local host files, so ERTS and PBeam give the same result on one host.
%% The probe runs `prim_socket:init/0`, which the boot runs in the
%% `on_load` of the preloaded `prim_socket`, for the protocol table of
%% `prim_net:getaddrinfo/2` (`prim_net.erl:452-469`).
lookups() ->
    ok = prim_socket:init(),
    In = #{family => inet, addr => {127,0,0,1}, port => 80},
    In6 = #{family => inet6, addr => {0,0,0,0,0,0,0,1}, port => 80,
            flowinfo => 0, scope_id => 0},
    Numeric = [numerichost, numericserv],
    [{Name, call(Fun)} || {Name, Fun} <-
        [{name_numeric, fun() -> net:getnameinfo(In, Numeric) end},
         {name_numeric6, fun() -> net:getnameinfo(In6, Numeric) end},
         {name_lookup, fun() -> net:getnameinfo(In) end},
         {name_flags, fun() -> net:getnameinfo(In, [dgram, nofqdn, namereqd | Numeric]) end},
         {name_idn, fun() -> net:getnameinfo(In, [idn | Numeric]) end},
         {name_improper, fun() -> prim_net:getnameinfo(In, [numerichost | x]) end},
         {name_bad_flag, fun() -> net:getnameinfo(In, [x]) end},
         {name_atom_flags, fun() -> prim_net:getnameinfo(In, x) end},
         {name_integer_flags, fun() -> prim_net:getnameinfo(In, 1) end},
         {name_loopback, fun() -> net:getnameinfo(In#{addr := loopback}, Numeric) end},
         {name_any, fun() -> net:getnameinfo(In#{addr := any}, Numeric) end},
         {name_broadcast, fun() -> net:getnameinfo(In#{addr := broadcast}, Numeric) end},
         {name_any6, fun() -> net:getnameinfo(In6#{addr := any}, Numeric) end},
         {name_loopback6, fun() -> net:getnameinfo(In6#{addr := loopback}, Numeric) end},
         {name_port, fun() -> net:getnameinfo(In#{port := 65616}, Numeric) end},
         {name_flow, fun() -> net:getnameinfo(In6#{flowinfo := 7, scope_id := 0}, Numeric) end},
         {name_bad_addr, fun() -> net:getnameinfo(In#{addr := {256,0,0,1}}, Numeric) end},
         {name_bad_addr6, fun() -> net:getnameinfo(In6#{addr := {1,2,3}}, Numeric) end},
         {name_bad_word6, fun() -> net:getnameinfo(In6#{addr := {0,0,0,0,0,0,0,65536}}, Numeric) end},
         {name_int_addr, fun() -> net:getnameinfo(In#{addr := 1}, Numeric) end},
         {name_int_flag, fun() -> net:getnameinfo(In, [1]) end},
         {name_bad_flow, fun() -> net:getnameinfo(In6#{flowinfo := x}, Numeric) end},
         {name_local_list, fun() -> net:getnameinfo(#{family => local, path => 1}) end},
         {name_unspec_int, fun() -> net:getnameinfo(#{family => unspec, addr => 1}) end},
         {name_unspec_bits, fun() -> net:getnameinfo(#{family => unspec, addr => <<1:1>>}) end},
         {name_bad_atom, fun() -> net:getnameinfo(In#{addr := x}, Numeric) end},
         {name_bad_atom6, fun() -> net:getnameinfo(In6#{addr := broadcast}, Numeric) end},
         {name_bad_port, fun() -> net:getnameinfo(In#{port := x}, Numeric) end},
         {name_bad_scope, fun() -> net:getnameinfo(In6#{scope_id := -1}, Numeric) end},
         {name_local, fun() -> net:getnameinfo(#{family => local, path => <<"/x">>}) end},
         {name_local_long, fun() -> net:getnameinfo(#{family => local, path => binary:copy(<<"x">>, 200)}) end},
         {name_unspec, fun() -> net:getnameinfo(#{family => unspec, addr => <<>>}) end},
         {name_native, fun() -> net:getnameinfo(#{family => 200, addr => <<1,2>>}) end},
         {name_native_long, fun() -> net:getnameinfo(#{family => 200, addr => binary:copy(<<1>>, 200)}) end},
         {name_family, fun() -> prim_net:getnameinfo(#{family => x}, undefined) end},
         {addr_localhost, fun() -> net:getaddrinfo("localhost") end},
         {addr_service, fun() -> net:getaddrinfo("127.0.0.1", "http") end},
         {addr_port, fun() -> net:getaddrinfo(undefined, "80") end},
         {addr_v6, fun() -> net:getaddrinfo("::1", "8080") end},
         {addr_no_service, fun() -> net:getaddrinfo("127.0.0.1", "pbeam-no-such-service") end},
         {addr_nul, fun() -> net:getaddrinfo([$1, $2, $7, $., $0, $., $0, $., $1, 0, $x]) end},
         {addr_atom_char, fun() -> net:getaddrinfo([a]) end},
         {addr_wide, fun() -> net:getaddrinfo([256]) end},
         {serv_name, fun() -> net:getservbyname("http") end},
         {serv_atom, fun() -> net:getservbyname(http, tcp) end},
         {serv_udp, fun() -> net:getservbyname("domain", udp) end},
         {serv_none, fun() -> net:getservbyname("pbeam-no-such-service") end},
         {serv_empty, fun() -> net:getservbyname("") end},
         {serv_wide, fun() -> net:getservbyname([256]) end},
         {serv_long, fun() -> net:getservbyname(lists:duplicate(256, $a)) end},
         {serv_long_proto, fun() -> prim_net:getservbyname("http", lists:duplicate(256, $a)) end},
         {port_any, fun() -> net:getservbyport(80) end},
         {port_wrap, fun() -> net:getservbyport(65616) end},
         {port_udp, fun() -> net:getservbyport(53, udp) end},
         {port_negative, fun() -> net:getservbyport(-1) end},
         {port_big, fun() -> net:getservbyport(1 bsl 32) end},
         {port_none, fun() -> net:getservbyport(80, x) end},
         {port_wide, fun() -> prim_net:getservbyport(80, [256]) end}]].

%% The external term format of each case of `main/0` and of
%% `net:gethostname/0`, in hexadecimal, one line for each. Each map is a
%% sorted list of its pairs: the external format writes the keys of a
%% small map in the order of their atom indexes, which depend on the run.
text() ->
    iolist_to_binary([[binary:encode_hex(term_to_binary(canonical(Case))), $\n]
                      || Case <- main() ++ [net:gethostname()]]).

canonical(Map) when is_map(Map) ->
    lists:sort([{Key, canonical(Value)} || {Key, Value} <- maps:to_list(Map)]);
canonical(List) when is_list(List) ->
    [canonical(Element) || Element <- List];
canonical(Tuple) when is_tuple(Tuple) ->
    list_to_tuple(canonical(tuple_to_list(Tuple)));
canonical(Term) ->
    Term.

%% Turn on the debug trace of the library.
debug_on() ->
    prim_net:debug(true).

call(Fun) ->
    try Fun() of
        Value -> {value, Value}
    catch
        Class:Reason -> {Class, Reason}
    end.
