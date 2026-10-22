-module(prim_socket).
-export([load/2]).
-export([nif_info/0, nif_info/1, nif_supports/0, nif_supports/1, nif_command/1, nif_open/2, nif_open/4, nif_bind/2, nif_bind/3, nif_connect/1, nif_connect/3, nif_listen/2, nif_accept/2, nif_peeloff/2, nif_send/4, nif_sendto/5, nif_sendmsg/5, nif_sendmmsg/4, nif_sendv/3, nif_sendfile/5, nif_sendfile/4, nif_sendfile/1, nif_recv/4, nif_recvfrom/4, nif_recvmsg/5, nif_recvmmsg/6, nif_close/1, nif_shutdown/2, nif_setopt/5, nif_getopt/3, nif_getopt/5, nif_sockname/1, nif_socknames/2, nif_peername/1, nif_peernames/2, nif_ioctl/2, nif_ioctl/3, nif_ioctl/4, nif_cancel/3, nif_finalize_close/1]).
-nifs([nif_info/0, nif_info/1, nif_supports/0, nif_supports/1, nif_command/1, nif_open/2, nif_open/4, nif_bind/2, nif_bind/3, nif_connect/1, nif_connect/3, nif_listen/2, nif_accept/2, nif_peeloff/2, nif_send/4, nif_sendto/5, nif_sendmsg/5, nif_sendmmsg/4, nif_sendv/3, nif_sendfile/5, nif_sendfile/4, nif_sendfile/1, nif_recv/4, nif_recvfrom/4, nif_recvmsg/5, nif_recvmmsg/6, nif_close/1, nif_shutdown/2, nif_setopt/5, nif_getopt/3, nif_getopt/5, nif_sockname/1, nif_socknames/2, nif_peername/1, nif_peernames/2, nif_ioctl/2, nif_ioctl/3, nif_ioctl/4, nif_cancel/3, nif_finalize_close/1]).

%% A stand-in for the prim_socket module with each function of the nif_funcs
%% table of its NIF library (prim_socket_nif.c:18696-18748). load/2 returns the result of
%% erlang:load_nif/2 for each load info, in order. An error does not load
%% the library, so the next load info runs the checks again.
load(Lib, Infos) ->
    [erlang:load_nif(Lib, Info) || Info <- Infos].

nif_info() -> erlang:nif_error(undef).
nif_info(_) -> erlang:nif_error(undef).
nif_supports() -> erlang:nif_error(undef).
nif_supports(_) -> erlang:nif_error(undef).
nif_command(_) -> erlang:nif_error(undef).
nif_open(_, _) -> erlang:nif_error(undef).
nif_open(_, _, _, _) -> erlang:nif_error(undef).
nif_bind(_, _) -> erlang:nif_error(undef).
nif_bind(_, _, _) -> erlang:nif_error(undef).
nif_connect(_) -> erlang:nif_error(undef).
nif_connect(_, _, _) -> erlang:nif_error(undef).
nif_listen(_, _) -> erlang:nif_error(undef).
nif_accept(_, _) -> erlang:nif_error(undef).
nif_peeloff(_, _) -> erlang:nif_error(undef).
nif_send(_, _, _, _) -> erlang:nif_error(undef).
nif_sendto(_, _, _, _, _) -> erlang:nif_error(undef).
nif_sendmsg(_, _, _, _, _) -> erlang:nif_error(undef).
nif_sendmmsg(_, _, _, _) -> erlang:nif_error(undef).
nif_sendv(_, _, _) -> erlang:nif_error(undef).
nif_sendfile(_, _, _, _, _) -> erlang:nif_error(undef).
nif_sendfile(_, _, _, _) -> erlang:nif_error(undef).
nif_sendfile(_) -> erlang:nif_error(undef).
nif_recv(_, _, _, _) -> erlang:nif_error(undef).
nif_recvfrom(_, _, _, _) -> erlang:nif_error(undef).
nif_recvmsg(_, _, _, _, _) -> erlang:nif_error(undef).
nif_recvmmsg(_, _, _, _, _, _) -> erlang:nif_error(undef).
nif_close(_) -> erlang:nif_error(undef).
nif_shutdown(_, _) -> erlang:nif_error(undef).
nif_setopt(_, _, _, _, _) -> erlang:nif_error(undef).
nif_getopt(_, _, _) -> erlang:nif_error(undef).
nif_getopt(_, _, _, _, _) -> erlang:nif_error(undef).
nif_sockname(_) -> erlang:nif_error(undef).
nif_socknames(_, _) -> erlang:nif_error(undef).
nif_peername(_) -> erlang:nif_error(undef).
nif_peernames(_, _) -> erlang:nif_error(undef).
nif_ioctl(_, _) -> erlang:nif_error(undef).
nif_ioctl(_, _, _) -> erlang:nif_error(undef).
nif_ioctl(_, _, _, _) -> erlang:nif_error(undef).
nif_cancel(_, _, _) -> erlang:nif_error(undef).
nif_finalize_close(_) -> erlang:nif_error(undef).
