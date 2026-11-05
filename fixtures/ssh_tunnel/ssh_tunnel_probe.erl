-module(ssh_tunnel_probe).
-export([main/0]).

%% Connect with public key authentication from PBeam to an `ssh:daemon/2`
%% on the pinned ERTS, open a TCP/IP tunnel with
%% `ssh:tcpip_tunnel_to_server/6`, and send data through it, as the Redbug
%% CLI does (server/lib/server/ssh_tunnel.ex:185, :195).
%%
%% The daemon and the echo server run on the pinned ERTS. The test starts
%% them and gives their ports in the environment, with the key directory.
%% The keys are generated in the test with `crypto:generate_key/1` and
%% written as a PEM and an authorized_keys file, so no key on disk is
%% needed. The client reads its key with the option `user_dir`, and the
%% daemon reads the host key and the authorized keys with `system_dir`
%% and `user_dir` (ssh_file.erl:1105-1126, 726-777, 356-367).
main() ->
    {ok, _} = application:ensure_all_started(ssh),
    Dir = os:getenv("PBEAM_SSH_DIR"),
    SshPort = list_to_integer(os:getenv("PBEAM_SSH_PORT")),
    EchoPort = list_to_integer(os:getenv("PBEAM_ECHO_PORT")),
    {ok, C} = ssh:connect("127.0.0.1", SshPort,
                          [{user_dir, Dir}, {user, "pbeam"},
                           {auth_methods, "publickey"},
                           {silently_accept_hosts, true},
                           {user_interaction, false}], 30000),
    {ok, Tunnel} = ssh:tcpip_tunnel_to_server(C, "127.0.0.1", 0,
                                              "127.0.0.1", EchoPort, 30000),
    {ok, S} = gen_tcp:connect("127.0.0.1", Tunnel, [binary, {active, false}], 5000),
    ok = gen_tcp:send(S, <<"hello">>),
    io:format("~p~n", [gen_tcp:recv(S, 0, 5000)]),
    halt().
