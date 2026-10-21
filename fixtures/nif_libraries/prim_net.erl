-module(prim_net).
-export([load/2]).
-export([nif_info/0, nif_command/1, nif_gethostname/0, nif_getnameinfo/2, nif_getaddrinfo/3, nif_getifaddrs/1, nif_get_adapters_addresses/1, nif_get_if_entry/1, nif_get_interface_info/1, nif_get_ip_address_table/1, nif_getservbyname/2, nif_getservbyport/2, nif_if_name2index/1, nif_if_index2name/1, nif_if_names/0]).
-nifs([nif_info/0, nif_command/1, nif_gethostname/0, nif_getnameinfo/2, nif_getaddrinfo/3, nif_getifaddrs/1, nif_get_adapters_addresses/1, nif_get_if_entry/1, nif_get_interface_info/1, nif_get_ip_address_table/1, nif_getservbyname/2, nif_getservbyport/2, nif_if_name2index/1, nif_if_index2name/1, nif_if_names/0]).

%% A stand-in for the prim_net module with each function of the nif_funcs
%% table of its NIF library (prim_net_nif.c:4997-5022). load/2 returns the result of
%% erlang:load_nif/2 for each load info, in order. An error does not load
%% the library, so the next load info runs the checks again.
load(Lib, Infos) ->
    [erlang:load_nif(Lib, Info) || Info <- Infos].

nif_info() -> erlang:nif_error(undef).
nif_command(_) -> erlang:nif_error(undef).
nif_gethostname() -> erlang:nif_error(undef).
nif_getnameinfo(_, _) -> erlang:nif_error(undef).
nif_getaddrinfo(_, _, _) -> erlang:nif_error(undef).
nif_getifaddrs(_) -> erlang:nif_error(undef).
nif_get_adapters_addresses(_) -> erlang:nif_error(undef).
nif_get_if_entry(_) -> erlang:nif_error(undef).
nif_get_interface_info(_) -> erlang:nif_error(undef).
nif_get_ip_address_table(_) -> erlang:nif_error(undef).
nif_getservbyname(_, _) -> erlang:nif_error(undef).
nif_getservbyport(_, _) -> erlang:nif_error(undef).
nif_if_name2index(_) -> erlang:nif_error(undef).
nif_if_index2name(_) -> erlang:nif_error(undef).
nif_if_names() -> erlang:nif_error(undef).
