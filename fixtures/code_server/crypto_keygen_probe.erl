-module(crypto_keygen_probe).
-export([main/0, post_quantum/0, unsupported_field/0, unsupported_unnamed/0]).

%% Write the results of crypto:generate_key/2,3, which call
%% rsa_generate_key_nif/2, ec_generate_key_nif/2, and
%% evp_generate_key_nif/2 (crypto.erl:3664-3735, rsa.c:453-543,
%% ec.c:353-517, evp.c:221-300). A new key is random, so the probe writes
%% its sizes and the result of a sign and verify with it. A key from a
%% given private key does not depend on a random value, so the probe
%% writes its bytes. An error writes its reason, which names the C file
%% and line of the failed check.
main() ->
    Msg = <<"abc">>,
    Rsa = [rsa(Bits, E) || {Bits, E} <- [{1024, 65537}, {512, 3}, {513, 65537},
                                        {512, <<3, 0, 0>>}]],
    RsaErrors = [catch_error(fun() -> crypto:generate_key(rsa, {Bits, E}) end)
                 || {Bits, E} <- [{255, 3}, {256, 65537}, {511, 65537}, {512, 1},
                                  {512, 2}, {512, 0}, {512, <<>>}, {512, <<0, 0, 3>>},
                                  {-1, 3}, {a, 3}, {4294967296, 3}, {512, a}]],
    Ec = [ec(Curve, Msg) || Curve <- [secp256r1, secp384r1, secp521r1, brainpoolP256r1,
                                      secp256k1, prime192v1,
                                      crypto_ec_curves:curve(secp256r1)]],
    EcGiven = [catch_error(fun() -> crypto:generate_key(ecdh, Curve, Priv) end)
               || {Curve, Priv} <- [{secp256r1, 1}, {secp521r1, <<1>>}, {secp256r1, 0},
                                    {secp256r1, <<>>},
                                    {secp256r1, binary:copy(<<255>>, 32)},
                                    {secp256r1, binary:copy(<<255>>, 33)},
                                    {secp256r1, <<0, 0, 7>>},
                                    {secp256r1, a}, {foo, undefined}, {foo, 1},
                                    {crypto_ec_curves:curve(secp256r1), a},
                                    {setelement(4, crypto_ec_curves:curve(secp256r1), a),
                                     undefined},
                                    {setelement(5, crypto_ec_curves:curve(secp256r1), none),
                                     undefined},
                                    {setelement(3, crypto_ec_curves:curve(secp256r1), a),
                                     undefined},
                                    {setelement(1, crypto_ec_curves:curve(secp256r1),
                                                {foo, <<1>>}), undefined},
                                    {{{prime_field, <<4>>}, {<<1>>, <<1>>, none}, <<4, 1, 1>>,
                                      <<5>>, <<1>>}, undefined}]],
    Evp = [evp(Type, Curve, Msg) || {Type, Curve} <- [{eddsa, ed25519}, {eddsa, ed448},
                                                      {eddh, x25519}, {eddh, x448},
                                                      {ecdh, x25519}]],
    EvpGiven = [catch_error(fun() -> crypto:generate_key(Type, Curve, Priv) end)
                || {Type, Curve, Priv} <- [{eddsa, ed25519, binary:copy(<<1>>, 32)},
                                           {eddsa, ed448, binary:copy(<<5>>, 57)},
                                           {eddh, x25519, binary:copy(<<1>>, 32)},
                                           {eddh, x448, binary:copy(<<2>>, 56)},
                                           {eddsa, ed25519, <<1, 2>>},
                                           {eddsa, ed25519, a},
                                           {foo, [], undefined},
                                           {"x25519", [], undefined},
                                           {x25519, [], binary:copy(<<1>>, 32)}]],
    io:format("~p~n", [[Rsa, RsaErrors, Ec, EcGiven, Evp, EvpGiven]]).

%% ML-DSA, ML-KEM, and SLH-DSA (evp.c:232-257). The Linux CI ERTS links
%% OpenSSL 3.0.x, which has none of them.
post_quantum() ->
    Msg = <<"abc">>,
    Sign = [begin
                {Pub, Priv} = crypto:generate_key(Type, []),
                {Pub2, Priv2} = crypto:generate_key(Type, [], Priv),
                {Type, byte_size(Pub), byte_size(Priv), {Pub2, Priv2} =:= {Pub, Priv},
                 crypto:verify(Type, none, Msg, crypto:sign(Type, none, Msg, Priv), Pub)}
            end || Type <- [mldsa44, mldsa65, mldsa87, slh_dsa_shake_128f,
                            slh_dsa_sha2_128f]],
    Kem = [begin
               {Pub, Priv} = crypto:generate_key(Type, []),
               {Type, byte_size(Pub), byte_size(Priv),
                crypto:generate_key(Type, [], Priv) =:= {Pub, Priv}}
           end || Type <- [mlkem512, mlkem768, mlkem1024]],
    Sizes = [begin
                 {Pub, Priv} = crypto:generate_key(Type, []),
                 {Type, byte_size(Pub), byte_size(Priv)}
             end || Type <- [slh_dsa_shake_192f, slh_dsa_sha2_256f]],
    Errors = [catch_error(fun() -> crypto:generate_key(Type, [], <<1>>) end)
              || Type <- [mldsa44, mlkem512]],
    io:format("~p~n", [{Sign, Kem, Sizes, Errors}]).

%% A curve over a characteristic-two field (ec.c:141-215).
unsupported_field() ->
    crypto:generate_key(ecdh, sect163k1).

%% A private key on a curve without a name (ec.c:465-482).
unsupported_unnamed() ->
    crypto:generate_key(ecdh, crypto_ec_curves:curve(secp256r1), 1).

rsa(Bits, E) ->
    {Pub, Priv} = crypto:generate_key(rsa, {Bits, E}),
    [Exp, N | _] = Priv,
    {Pub =:= lists:sublist(Priv, 2), length(Priv), Exp, byte_size(N),
     crypto:verify(rsa, sha256, <<"abc">>, crypto:sign(rsa, sha256, <<"abc">>, Priv), Pub)}.

ec(Curve, Msg) ->
    {Pub = <<First, _/binary>>, Priv} = crypto:generate_key(ecdh, Curve),
    Sig = crypto:sign(ecdsa, sha256, Msg, [Priv, Curve]),
    {byte_size(Pub), First, byte_size(Priv),
     crypto:verify(ecdsa, sha256, Msg, Sig, [Pub, Curve])}.

evp(eddsa, Curve, Msg) ->
    {Pub, Priv} = crypto:generate_key(eddsa, Curve),
    {byte_size(Pub), byte_size(Priv), crypto:generate_key(eddsa, Curve, Priv) =:= {Pub, Priv},
     crypto:verify(eddsa, none, Msg, crypto:sign(eddsa, none, Msg, [Priv, Curve]), [Pub, Curve])};
evp(Type, Curve, _Msg) ->
    {Pub, Priv} = crypto:generate_key(Type, Curve),
    {byte_size(Pub), byte_size(Priv), crypto:generate_key(Type, Curve, Priv) =:= {Pub, Priv}}.

catch_error(F) ->
    try F() catch error:Reason -> Reason end.
