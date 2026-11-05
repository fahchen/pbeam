-module(ssh_keys).
-export([write/1]).

%% Write a host key, a client private key, and the authorized_keys file of
%% the client public key into `Dir`, as the test generates them. The keys
%% are RSA keys of 2048 bits, the pinned default of
%% `ssh:daemon/2`. The PEM of a key is the `RSAPrivateKey` entry
%% (public_key:pem_entry_encode/2). The authorized_keys file is the
%% OpenSSH public key of the client key with a newline
%% (ssh_file:encode/2, openssh_key).
write(Dir) ->
    ok = filelib:ensure_path(Dir),
    Host = public_key:generate_key({rsa, 2048, 65537}),
    HostPem = public_key:pem_encode([public_key:pem_entry_encode('RSAPrivateKey', Host)]),
    ok = file:write_file(filename:join(Dir, "ssh_host_rsa_key"), HostPem),
    Client = public_key:generate_key({rsa, 2048, 65537}),
    ClientPem = public_key:pem_encode([public_key:pem_entry_encode('RSAPrivateKey', Client)]),
    ok = file:write_file(filename:join(Dir, "id_rsa"), ClientPem),
    Pub = ssh_file:extract_public_key(Client),
    Auth = ssh_file:encode([{Pub, []}], auth_keys),
    ok = file:write_file(filename:join(Dir, "authorized_keys"), Auth).
