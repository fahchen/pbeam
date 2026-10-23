-module(crypto_sign_probe).
-export([main/0, host/0, post_quantum/0, unsupported_name/0, unsupported_curve/0]).

%% Write the results of crypto:sign/5 and crypto:verify/6, which call
%% pkey_sign_nif/5 and pkey_verify_nif/6 (crypto.erl:3285-3389,
%% pkey.c:762-1122). RSA PKCS #1 v1.5, RSA PSS with a salt of 0 bytes, and
%% EdDSA signatures do not depend on a random value, so the probe writes
%% their bytes. A DSA, an ECDSA, or a PSS signature with a salt has a
%% random value, so the probe writes only the verify result of each one.
%% An error writes its class and reason, which names the C file and line
%% of the failed check (crypto.erl:761-775, common.c:29-90).
main() ->
    Msg = <<"abc">>,
    RsaSigs = [crypto:sign(rsa, Type, Data, Key, Opts)
               || Type <- [sha, sha256, sha512, md5, sha3_256],
                  Data <- [Msg, [<<"a">>, "b", [$c]]],
                  Key <- [rsa_priv(), rsa_priv3(), rsa_priv_int()],
                  Opts <- [[], [{rsa_padding, rsa_pkcs1_padding}]]],
    RsaDigest = [crypto:sign(rsa, sha256, {digest, crypto:hash(sha256, Msg)}, rsa_priv()),
                 crypto:sign(rsa, none, Msg, rsa_priv()),
                 crypto:sign(rsa, none, {digest, Msg}, rsa_priv())],
    Pss0 = [crypto:sign(rsa, Type, Msg, rsa_priv(),
                        [{rsa_padding, rsa_pkcs1_pss_padding}, {rsa_pss_saltlen, 0} | Mgf])
            || Type <- [sha, sha256], Mgf <- [[], [{rsa_mgf1_md, sha512}]]],
    PssVerify = [crypto:verify(rsa, sha256, Msg,
                               crypto:sign(rsa, sha256, Msg, rsa_priv(), Opts), rsa_pub(), Opts)
                 || Opts <- [[{rsa_padding, rsa_pkcs1_pss_padding}],
                             [{rsa_padding, rsa_pkcs1_pss_padding}, {rsa_pss_saltlen, -1}],
                             [{rsa_padding, rsa_pkcs1_pss_padding}, {rsa_pss_saltlen, -2}],
                             [{rsa_padding, rsa_pkcs1_pss_padding}, {rsa_pss_saltlen, 20}],
                             [{rsa_mgf1_md, sha}, {rsa_padding, rsa_pkcs1_pss_padding}]]],
    Sig = crypto:sign(rsa, sha256, Msg, rsa_priv()),
    RsaVerify = [crypto:verify(rsa, sha256, Msg, Sig, rsa_pub()),
                 crypto:verify(rsa, sha256, <<"abd">>, Sig, rsa_pub()),
                 crypto:verify(rsa, sha, Msg, Sig, rsa_pub()),
                 crypto:verify(rsa, sha256, Msg, <<1, 2, 3>>, rsa_pub()),
                 crypto:verify(rsa, sha256, Msg, Sig, [65537, rsa_n()]),
                 crypto:verify(rsa, sha256, Msg, Sig, rsa_pub(),
                               [{rsa_padding, rsa_pkcs1_pss_padding}])],
    NoPad = crypto:sign(rsa, none, binary:copy(<<1>>, 128), rsa_priv(),
                        [{rsa_padding, rsa_no_padding}]),
    NoPadVerify = crypto:verify(rsa, none, binary:copy(<<1>>, 128), NoPad, rsa_pub(),
                                [{rsa_padding, rsa_no_padding}]),
    X931 = crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_padding, rsa_x931_padding}]),
    Ed = [crypto:sign(eddsa, Type, Data, [ed_priv(), ed25519])
          || Type <- [none, sha256, sha512], Data <- [Msg, ["a", <<"bc">>], <<>>]],
    Ed448 = [crypto:sign(eddsa, none, Msg, [binary:copy(<<5>>, 57), ed448]),
             crypto:verify(eddsa, none, Msg,
                           crypto:sign(eddsa, none, Msg, [binary:copy(<<5>>, 57), ed448]),
                           [ed448_pub(), ed448])],
    EdSig = crypto:sign(eddsa, none, Msg, [ed_priv(), ed25519]),
    EdVerify = [crypto:verify(eddsa, none, Msg, EdSig, [ed_pub(), ed25519]),
                crypto:verify(eddsa, sha256, Msg, EdSig, [ed_pub(), ed25519]),
                crypto:verify(eddsa, none, <<"abd">>, EdSig, [ed_pub(), ed25519]),
                crypto:verify(eddsa, none, Msg, <<1>>, [ed_pub(), ed25519])],
    Def = crypto_ec_curves:curve(secp256r1),
    Explicit = crypto:verify(ecdsa, sha256, Msg, crypto:sign(ecdsa, sha256, Msg, [ec_priv(), Def]),
                             [ec_pub(), Def]),
    Ec = [crypto:verify(ecdsa, Type, Msg, crypto:sign(ecdsa, Type, Msg, [Priv, Curve]),
                        [ec_pub(), Curve])
          || Type <- [sha, sha256, sha512],
             Priv <- [ec_priv(), ec_priv_int()],
             Curve <- [secp256r1, prime256v1]],
    EcSig = crypto:sign(ecdsa, sha256, Msg, [ec_priv(), secp256r1]),
    EcVerify = [crypto:verify(ecdsa, sha256, <<"abd">>, EcSig, [ec_pub(), secp256r1]),
                crypto:verify(ecdsa, sha256, {digest, crypto:hash(sha256, Msg)}, EcSig,
                              [ec_pub(), secp256r1]),
                crypto:verify(ecdsa, sha256, Msg, <<1, 2>>, [ec_pub(), secp256r1])],
    Dss = [crypto:verify(dss, Type, Msg, crypto:sign(dss, Type, Msg, Priv), dss_pub())
           || Type <- [sha, sha256], Priv <- [dss_priv(), dss_priv_int()]],
    %% crypto:sign/5 gives dss and none to the NIF as the algorithm sha and
    %% the type {digest, Data} (crypto.erl:3392-3393), so it fails.
    DssDigest = [bad_reason(fun() -> crypto:sign(dss, none, crypto:hash(sha, Msg), dss_priv()) end),
                 bad_reason(fun() -> crypto:verify(dss, none, crypto:hash(sha, Msg), <<1>>,
                                                   dss_pub()) end)],
    DssVerify = crypto:verify(dss, sha, <<"abd">>, crypto:sign(dss, sha, Msg, dss_priv()),
                              dss_pub()),
    io:format("~p~n", [[RsaSigs, RsaDigest, Pss0, PssVerify, RsaVerify, NoPad, NoPadVerify,
                        X931, Ed, Ed448, EdVerify, Explicit, Ec, EcVerify, Dss, DssDigest, DssVerify,
                        errors()]]).

errors() ->
    Msg = <<"abc">>,
    [bad_reason(Fun)
     || Fun <- [fun() -> crypto:sign(rsa, nosuch, Msg, rsa_priv()) end,
                                fun() -> crypto:sign(ecdsa, none, Msg, [ec_priv(), secp256r1]) end,
                fun() -> crypto:sign(rsa, sha256, {digest, <<1>>}, rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, {digest, a}, rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, {other, <<1>>}, rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, {digest, <<1>>, 2}, rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, a, rsa_priv()) end,
                fun() -> crypto:sign(rsa, none, a, rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), a) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, [ec_priv(), secp256r1], [a]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [a]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{a, b, c}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_mgf1_md, 1}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_mgf1_md, nosuch}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_padding, a}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_pss_saltlen, -3}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_pss_saltlen, a}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{a, b}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(),
                                     [{rsa_padding, rsa_pkcs1_pss_padding},
                                      {rsa_pss_saltlen, 1000}]) end,
                fun() -> byte_size(crypto:sign(rsa, sha256, Msg, rsa_priv(),
                                               [{rsa_padding, rsa_pkcs1_pss_padding},
                                                {rsa_mgf1_md, none}])) end,
                fun() -> crypto:sign(rsa, none, Msg, rsa_priv(),
                                     [{rsa_padding, rsa_pkcs1_pss_padding}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(),
                                     [{rsa_padding, rsa_no_padding}]) end,
                fun() -> crypto:sign(rsa, none, binary:copy(<<1>>, 200), rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, Msg, [1, 2]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, [<<1>>, <<2>>, <<3>>, <<4>>]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, a) end,
                fun() -> crypto:sign(rsa, sha256, Msg, [a, <<1>>, <<1>>]) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, [a, secp256r1]) end,
                fun() -> crypto:sign(eddsa, none, Msg, [<<1, 2>>, ed25519]) end,
                fun() -> crypto:sign(eddsa, none, Msg, [ed_priv(), ed25519], [a]) end,
                fun() -> crypto:sign(dss, sha, Msg, [<<1>>, <<2>>]) end,
                fun() -> crypto:sign(dss, sha, Msg, dss_priv() ++ [<<1>>]) end,
                fun() -> crypto:verify(rsa, sha256, Msg, a, rsa_pub()) end,
                fun() -> crypto:verify(rsa, sha256, Msg, <<1>>, [<<1>>]) end,
                fun() -> crypto:verify(rsa, sha256, Msg, <<1>>, rsa_pub(), a) end,
                fun() -> crypto:verify(rsa, sha256, Msg, <<1>>, rsa_pub(),
                                       [{rsa_padding, rsa_pkcs1_pss_padding},
                                        {rsa_pss_saltlen, 1000}]) end,
                fun() -> crypto:verify(ecdsa, sha256, Msg, <<1>>, [<<4, 1>>, secp256r1]) end,
                fun() -> crypto:verify(eddsa, none, Msg, <<1>>, [<<1>>, ed25519]) end,
                fun() -> crypto:verify(dss, sha, Msg, <<1>>, [<<1>>]) end,
                fun() -> crypto:sign(rsa, blake2b, Msg, rsa_priv()) end,
                fun() -> crypto:verify(rsa, blake2b, Msg, <<1>>, rsa_pub()) end,
                fun() -> crypto:verify(rsa, sha256, Msg, <<1>>, rsa_pub(),
                                       [{rsa_padding, rsa_no_padding}]) end,
                fun() -> crypto:sign(rsa, 1, Msg, rsa_priv()) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{1, 2}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(), [{rsa_pss_saltlen, 1 bsl 40}]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv(),
                                     [{rsa_padding, rsa_pkcs1_padding} | a]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, [<<1>>, <<1>>]) end,
                fun() -> crypto:sign(rsa, sha256, Msg, rsa_priv() ++ [<<1>>]) end,
                fun() -> crypto:verify(rsa, sha256, Msg, <<1>>, rsa_priv3()) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, a) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, {a, b, c}) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, {{a, b}, <<1>>}) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, {{{1, 2, 3, 4}, x}, <<1>>}) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg, {{{a, b, c, d, e}, x}, <<1>>}) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg,
                                     [<<1>>, {{foo, 1}, {<<1>>, <<1>>, none}, <<4>>, <<1>>, <<1>>}]) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg,
                                     [<<1>>, {{prime_field, <<7>>}, a, <<4>>, <<1>>, <<1>>}]) end,
                fun() -> crypto:sign(ecdsa, sha256, Msg,
                                     [<<1>>, {{prime_field, <<7>>}, {<<1>>, <<1>>, none},
                                              <<4>>, <<1>>, none}]) end,
                fun() -> crypto:sign(eddsa, none, Msg, [ed_priv()]) end,
                fun() -> crypto:sign(eddsa, none, Msg, [ed_priv(), foo]) end,
                fun() -> crypto:sign(eddsa, none, Msg, [ed_priv(), ed25519, x]) end,
                fun() -> crypto:sign(eddsa, none, Msg, [a, ed25519]) end,
                fun() -> crypto:sign(eddsa, none, Msg, a) end,
                fun() -> crypto:sign(rsa, sha256, Msg, #{engine => a}) end,
                fun() -> crypto:sign(rsa, sha256, Msg, #{}) end,
                fun() -> crypto:sign(rsa, sha256, Msg, #{engine => a, key_id => "k"}) end,
                fun() -> crypto:verify(rsa, sha256, Msg, <<>>, #{engine => a, key_id => <<"k">>,
                                                               password => "p"}) end,
                fun() -> crypto:sign(foo, sha256, Msg, <<1>>) end,
                fun() -> crypto:sign(foo, sha256, Msg, [1]) end,
                fun() -> crypto:sign(foo, sha256, Msg, {seed, <<1>>}) end,
                fun() -> crypto:sign(foo, sha256, Msg, {expandedkey, <<1>>}) end,
                fun() -> crypto:sign(foo, sha256, Msg, {a, b}) end,
                fun() -> crypto:sign(foo, none, Msg, <<1>>) end,
                fun() -> crypto:sign(list_to_atom(lists:duplicate(39, $a)), sha256, Msg, <<1>>) end,
                fun() -> crypto:sign(list_to_atom(lists:duplicate(40, $a)), sha256, Msg, <<1>>) end,
                fun() -> crypto:verify(foo, sha256, Msg, <<1>>, {seed, <<1>>}) end,
                fun() -> crypto:sign("foo", sha256, Msg, <<1>>) end,
                fun() -> crypto:sign('', sha256, Msg, <<1>>) end,
                fun() -> crypto:verify(1, sha256, Msg, <<1>>, <<1>>) end,
                fun() -> crypto:verify(list_to_atom(lists:duplicate(40, $a)), sha256, Msg,
                                       <<1>>, <<1>>) end]].

%% Write the result that depends on the libcrypto of the host: md4 is in
%% the legacy provider of OpenSSL 3, so its digest fetch can fail
%% (digest.c:186-188, pkey.c:283-284).
host() ->
    io:format("~p~n", [[bad_reason(fun() -> crypto:sign(rsa, Type, <<"abc">>, rsa_priv()) end)
                        || Type <- [md4, shake128]]]).

rsa_e() -> 65537.
rsa_n() -> 137996549326416731247446582927660627685427561547370507664206695462028684216104950527143568251764497054111749247400169456499517953239881984788043709864110375204800488302531664624395652043443730028525995049022133893851015706064724433607688147489589273014444821414517757577102222692099636199710004466025884156489.
rsa_d() -> 55851785569727082355593338299833653498877978394555788574287541345046475255690431553664085140883062763939669938924416662856855115578800824671603207411775441033024889488112155708093085575585623342620255523643896020057663197868793986393957876992031106962003940913039187651143134701117134795069152789584606415173.

rsa_priv_int() ->
    [rsa_e(), rsa_n(), rsa_d(),
     11996456636308674419821418027877463945391404144896985723225449517125624999697284264274707812580055473949895171253998693944246324914354600867560751495360967,
     11503109085457291501216123842851478182488005782809791358322821137675680925434295117845093680834072303490723197616072547105254177599769163912913907514125167,
     3010783507850604648629364843104330789840819924245321286839681304571193066436073234642879504727356339709292091136087561499534057893881387232702736478564737,
     378071882601812806408891630033448037064241031114825069591640702664958980627515322395567874460481739196469441196042239749526488831498280041326526340623245,
     5944910770378864987620126920169673991432008057696649387411936692460670740920407646551401075874139130403061788317641596206522733066882562072078557887873521].

rsa_priv() -> [unsigned(I) || I <- rsa_priv_int()].
rsa_priv3() -> lists:sublist(rsa_priv(), 3).
rsa_pub() -> lists:sublist(rsa_priv(), 2).

ed_priv() -> binary:copy(<<9>>, 32).
ed_pub() -> <<253,23,36,56,90,160,199,91,100,251,120,205,96,47,161,217,145,253,235,247,107,19,197,142,215,2,234,200,53,233,246,24>>.

ed448_pub() -> <<244,192,238,207,199,241,125,150,8,35,24,4,32,251,128,116,216,130,49,122,57,167,25,72,13,59,166,231,245,101,252,81,138,79,245,78,247,220,165,223,93,233,187,55,64,56,76,132,236,172,38,92,129,57,140,52,0>>.

ec_priv() -> binary:copy(<<7>>, 32).
ec_pub() -> <<4,30,24,83,47,212,117,76,2,243,4,29,156,117,206,179,59,131,255,216,26,199,206,79,232,130,204,177,201,139,197,137,110,164,108,49,28,78,47,244,13,217,106,54,83,230,228,84,69,211,45,254,72,110,206,215,92,122,144,198,161,136,129,192,163>>.

dss_priv_int() ->
    [165434756564013824650015218052440634313195102542221120366404998823236092538400212541009158745664616557541678040968734707812069491690384859131003113036102001327793451476966299818785095129620415509673490696088502111835375133850214564409156006461592975499089618971293589558017316194958628752447916166243721381409,
     1296757918358869538022243437569295704072805812853,
     159372385753476594635607433035292575700820355515300825989570558755506895379859812157657076548100806648268769926723280841064652772764185051774417963298227645107103179770941296499337460168840797665649073684287084574663265626719016701049656602978518219596076761192463545731186722647101910335015486199975316963157,
     216703504675533568272751657139108645003966993956].
dss_priv() -> [unsigned(I) || I <- dss_priv_int()].
dss_pub() ->
    lists:sublist(dss_priv(), 3)
        ++ [unsigned(77075403403905894123029294071648411094584215931570619720590351811049809614423268451713067499276900897834319764236975621827757187260493415172919582141220615453472341502037984657230410101191152346779824556443195977202620498817524046029865419600331645290055154220864147024044497894407239566388654255045782355589)].

bad_reason(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

%% Sign and verify with an algorithm or a curve that PBeam does not
%% implement, which stops the run with a runtime error.
unsupported_name() ->
    crypto:sign(dh, sha256, <<"abc">>, <<1>>).

unsupported_curve() ->
    crypto:sign(ecdsa, sha256, <<"abc">>, [<<1>>, sect163k1]).

%% ML-DSA and SLH-DSA need OpenSSL 3.5. The ML-DSA public keys are of the
%% seed <<0:256>>, from `openssl genpkey -pkeyopt hexseed:00...` (OpenSSL
%% 3.6.2). The SLH-DSA private keys are from crypto:generate_key/2 of the
%% pinned macOS ERTS; the public key is the second half of the private key.
%% ML-DSA and SLH-DSA signatures are randomized, so the probe gives sizes and
%% verify results.
post_quantum() ->
    Msg = <<"pbeam">>,
    Seed = {seed, <<0:256>>},
    MlDsa = [sign_and_verify(A, Seed, ml_dsa_public_key(A)) || A <- [mldsa44, mldsa65, mldsa87]],
    SlhDsa = [sign_and_verify(A, Key, binary:part(Priv, byte_size(Priv) div 2, byte_size(Priv) div 2))
              || {A, Priv} <- slh_dsa_private_keys(),
                 Key <- [Priv, {expandedkey, Priv}]],
    Pub44 = ml_dsa_public_key(mldsa44),
    Errors = [catch_error(F) || F <- [
        fun() -> crypto:sign(slh_dsa_sha2_128f, none, Msg, {seed, <<0:256>>}) end,
        fun() -> crypto:sign(mldsa44, none, Msg, {foo, <<0:256>>}) end,
        fun() -> crypto:sign(mldsa44, none, Msg, {seed, <<0:248>>}) end,
        fun() -> crypto:sign(mldsa44, none, Msg, {seed, a}) end,
        fun() -> crypto:sign(mldsa44, none, Msg, <<1>>) end,
        fun() -> crypto:sign(mldsa44, none, Msg, a) end,
        fun() -> crypto:verify(mldsa44, none, Msg, <<1>>, <<1>>) end,
        fun() -> crypto:verify(mldsa44, none, Msg, <<1>>, {seed, <<0:256>>}) end,
        fun() -> crypto:verify(mldsa44, none, Msg, <<1>>, Pub44) end,
        fun() -> crypto:sign(mldsa44, none, Msg, Seed, [{rsa_padding, rsa_no_padding}]) end
    ]],
    Digest = crypto:sign(mldsa44, sha256, Msg, Seed),
    Hashes = [crypto:verify(mldsa44, sha256, Msg, Digest, Pub44),
              crypto:verify(mldsa44, none, crypto:hash(sha256, Msg), Digest, Pub44),
              crypto:verify(mldsa44, none, {digest, crypto:hash(sha256, Msg)}, Digest, Pub44)],
    io:format("~p~n", [{MlDsa, SlhDsa, Errors, Hashes}]).

sign_and_verify(Algorithm, Priv, Pub) ->
    Sig = crypto:sign(Algorithm, none, <<"pbeam">>, Priv),
    {Algorithm, byte_size(Sig),
     crypto:verify(Algorithm, none, <<"pbeam">>, Sig, Pub),
     crypto:verify(Algorithm, none, [<<"pbea">>, $m], Sig, Pub),
     crypto:verify(Algorithm, none, <<"other">>, Sig, Pub)}.

catch_error(F) ->
    try F() catch error:Reason -> Reason end.

slh_dsa_private_keys() ->
    [
     {slh_dsa_shake_128s, base64:decode(<<"pVthDjb0JSNZA8LG2WcMwjQC1N2qbpNQzwzjFoEZSBJRgottbeherSTTMAxLKUEzBHVFx5BTdxwgH4HEZNgr3w==">>)},
     {slh_dsa_shake_128f, base64:decode(<<"fxJbFXfSzIxdcdCcEJjafN/biipjhOosnA/w9Fx8gW7is+q+vI/+Kdi4BAAwzwBPAoS65X1iPMXDHzws5Rc5og==">>)},
     {slh_dsa_sha2_128s, base64:decode(<<"ldeEMVfMNFjXXRk8AdU9SEV5olOtqs0l/nDzu19FtckacbBirOf6Z5xNPHvkXheKZGgHlZZiC043Z9hP+XmLLw==">>)},
     {slh_dsa_sha2_128f, base64:decode(<<"4/Sjje7Lfy5G9saksqFbLtbqoCkx3p/R0cnzbH0F+CrSX2Vun+CsXqIk6qPC+yRuh0ewvrka9VzZs9X3vS+RiA==">>)},
     {slh_dsa_shake_192s, base64:decode(<<"c5p19menCKZZYFdSa8OX96/0ffeINh4tBcYI7ueI3eeUuFVwFemnppHYKZyfHnHHOxzu2xPEBScj54PMPQOrrtiV8rtPaEpGnMgfOwAh7BqZhgkZ8poLTWNE84aikroG">>)},
     {slh_dsa_shake_192f, base64:decode(<<"iBTwJVpshPmkJED6G7DgEQLyZBrAYlhQ+zUyeNlIqc4BoXsG1auDlXYgzYNkR/yHJj7m0KFdpSJwziHLS82zE1gIzuUbp2b6ApQyI1tjJrcado8zDaupZi6NV4ERQJh+">>)},
     {slh_dsa_sha2_192s, base64:decode(<<"3xV3JOlj6C9wUUwhTUsdhSZXha83kT9T/EE0aViF/s4I4M2QZJ1kto43Bd3XWUjLfA9TediKKgv7J6XfCd+RPL2/EYGAZN7EXVGE0qUg7woj5BpCu88trZo7Kv4MFfCd">>)},
     {slh_dsa_sha2_192f, base64:decode(<<"UNv3noblETQG6CtX0L7FjY9pluvz+p554OwIut1nMdnYP+TQLtPSd9InHKAAZN8h4BWEIq10C9kNu+67c9s9M04f3TQ6uqIE5ZVjnbpkavzyGzByvgfqwsswT76VBDaC">>)},
     {slh_dsa_shake_256s, base64:decode(<<"atDc9RRsfTygdlhNXBU5p8EOmsKPgnZPshIE/jh/J4qkAsR3+h3jRKE+wyEIXjD4eqhE/KfO4Ff1rMgatFzcVjNurq/RPeyf6p/dt16lWiYFwKh/WDJqI4zdyq+/rLWxbxu80wZFeu/KGcISc04MGqmP8kjxAfWdR/Hwqd0/VO8=">>)},
     {slh_dsa_shake_256f, base64:decode(<<"wEmelZL8Y7WlIAZ60/23m4qqngyDhz4nh3QrGr82Td53Fd+3WmXEBwXcSlnWlILs7xcKB4Laxi9hkEYOXPdkMGBsoBPVuyI65U7+FdvPmURpq4mte+HskQkxz6zB09oklQdFCCvAkxIpZxZiZdRlxAN13Z/bjB9nxuutyMFLnWU=">>)},
     {slh_dsa_sha2_256s, base64:decode(<<"O31AsZ62okibwdPR5l/0UhyETUcVdQ8sB1RmazZu/yIGEPegBcA9Z7y3mKPHh3smQaborcssPo8hn8BJlFY/5tZEwjbdsEI/xDwBN3SMa6RHYYt5XQHam5Tmd62Ncbu650CC6twKgpCq6smQCBrS5Y3iTCg3168JghKyaXc6n8s=">>)},
     {slh_dsa_sha2_256f, base64:decode(<<"RymR4/aia8YyvOECY+3ZLkL0Nmo8Cgol0m936aTtjIjX1rtX4xj0A7p3poe7KTpID0HU3j2yXbAPFM7U/FOggj+yhrLWeuk3fDrz+FG5b8QooCQZzs5tzyTSCL9mpE97iuG9fNjb3OVOwRCV6w8eHXyOEhS6ej47iw5QYUuF6CY=">>)}
    ].

ml_dsa_public_key(mldsa44) ->
    base64:decode(<<
        "unH59k4RuutY+pxvu24U5h8YZD2rSVtHU5qRZsoBmBMcRPgmu9VuNOVdteXi1zNI"
        "XjnqJg/GAAxepLqA00Vc3lO0bzRIKu39VFD8Lhuk8l0V+cFEJC+zm7UihxiQMMUE"
        "mOFxe3x1ixkKZ0jqmqP3rKryx8tSbtcXyfea64QhT6XNje2SoMP6FViBDxLHBQo2"
        "dwjRls0k5a+XSQSu2OTOiHLoaWsLe8pQ5FLNfTDqmkrawDEdZyxr3oSWJAsHQxRj"
        "cIiVzZuvwxYy1zl2STiP2vy/fTBaPemkleynQzqPg7oPCyXEE8bjnJbrfWkbNNN8"
        "438e6tHPIX4l7zTuzz98YPhLjt/d6EBdT4MldsYe+Y4KLyjaGHcAlTkk9oa5RhRw"
        "W89T0z/t1DSO3dvfKLUGXh8gd1BD6Fz5MfgpF5NjoafnQEqDjsAAhrCXY4b+Y3yY"
        "JEdX4/dp3dRGdHG/rWcPmgX4JG7lCnser4f8QGnDriqiAzJYEXeS8LzUngg/0bx0"
        "lqv/KcyU5IaLISFO0xZSU5mmEPvdSoDnyAcV8pV44qhLtAvd29n0ehG259oRihtl"
        "jTWeiu9V60a1N2tbZVl5mEqSK+6/xZvNYA1TCdzNctvweH24unV7U3wer9XA9Q6k"
        "vJWDVJ4oKaQsKMrCSMlteBJMRxWbGK7ddUq6F7GdQw+3j2M+qdJvVKm9UPjY9rc1"
        "lPgol25+oJxTu7nxGlbJUH+4m5pevAN6NyZ6lfhbjWTKlxkrEKZvQXs/Yf6cpXEw"
        "pI/ZJeriq1UC1XHIpRkDwdOY9MH3an4RdDl2r9vGl/IwlKPNdh/5aF3jLgn7PCit"
        "1FNJAwC8fIncAXgAlgcXIpRXdfJk4bBiO89GGccSyDh2EgXYdpG3XvNgGWy7npuS"
        "oNTE7WIyblAk13UQuO4sdCbMIuriCdyfE73mvwj15xgb07RZRQtFGlFTmnFcIdZ9"
        "0zDrWXDbANntv7KCKwNvoTuv64bY3HiGbj+NQ+U9eMylWVpvr4hrXcES8c9K3PqH"
        "WADZC0iIOvlzFv4VBoc/wVflcOrL/SIoaNFCNBAZZq+2v5lAgpJTqVOtqJ/HVrao"
        "SfcKy5g45p+qULunXj6Jwq21fobQiKubBKKOZwcJFyJD7F4ACKXOrz+HIvSHMCWW"
        "/9dVrRuCpJw0s0aVFbRqopDNhu446nqb4/EDYQM1tTHMozPd/jKxRRD0sH75X8Zo"
        "ToxFSpLBDbtdWcenxj+zBf6IGWfZnmaetjKEBYJWC7QDQx1A91pJVJCEgieCkoIf"
        "TqkeQuePpIyu48g2FG3P1zjRF+kumhUTfSjo5qS0YiZQy0E1BMs6M11EvuxXRsHC"
        "lLHoy5nLYI2Sj4zjVjYyxSHyPRPGGo9hwB34yWxzYNtPPGiqXS/dNCpi/zRZwRY4"
        "lCGrQ+hYTEWIK1Dm5OlttvC4/eiQ1dv63NiGkLRJ5kJA3bICN0fzCDY+MBqnd1cW"
        "n8YVBijVkgtaoascjL9EywDgJdeHnXK0eeOvUxHHhXJVkNqcibn8O4RQdpVU60TS"
        "A+uiu675ytIjcBHC6kTv8A8pmkj/4oypPd+F92YIJC741swkYQoeIHj8rE+ThcMU"
        "kF7KqC5VORbZTRp8HsZSqgiJcIPaouuxd1+8Rxrid3fXkE6p8bkrysPYoxWEJgh7"
        "ZFsRCPDWX+yTeJwFN0PKFP1j0F6YtlLfK5wv+c4F8ZQHA/+yc/gODicy7KmWDZgb"
        "TP07e7gEWzw4MFRrndjbDQ=="
    >>);
ml_dsa_public_key(mldsa65) ->
    base64:decode(<<
        "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"
    >>);
ml_dsa_public_key(mldsa87) ->
    base64:decode(<<
        "5F/8jMc9uIXcZi5ioYzY44AylxF/pWWIFKmFtf8dt7Roz8gruSnx2Gt37RT1rham"
        "U2h3LOUZEkEBBeBFaXWukf22Q7US8STV5gvWi4x+Mf4Bx7DcZa5HBQHMVlpuHfz8"
        "/RJWVDPEr+3VEYIeLpYQxFJ14oNt7jXO1p1++mcv0eQxi+9etuiX6LRRqiAt7QQr"
        "Kq73envj9pkUbaIpqL2z/6SWRFln51IXv7yQSPmVZEPYcx+DPrMN4Q2slv/+fPZe"
        "oERcPjHoYB4TO+ahAHZP4xluJncmRB8xdR+/mm9YgGRPTnJ15X3isPEF5NsFXVDd"
        "HJyTT931NbjeKLDHTARJ8iLNLtC7j7x3XM7oyUBmW0D3EvT34AdQ6eHkzZz/JdGU"
        "XD6bylPM1PEu7nWBhW69aPJoRZVuPnvrdh8P51vdMb/i+gGBEzl7OHvVnWKmi4r3"
        "+iRauTLmn3eOLO79ITBPu4CZ6hPY6lfBgTGXovda4lEHW1Ha04+FNmnp1fmKNlUJ"
        "iUGZOhWUhg+6cf5TDuXCn1jyl4r2iMy3Wlg4o1nBEumOJahYOsjawfhh/Vjir7pd"
        "5aUuAgkE9bQrwIdONb788+YRloR2jzbgCPBHEhd86+YnYHOB5W6q7hYcFym43lHb"
        "3kdNSMxoJJ6icWK4eZPmDITtbMZCPLNnbZ61CyyrWjoEnvExOB1iP6b7y8nbHnzA"
        "JeoEGLna0sxszU6V+izsJP7spwMYp1Fxa3IT9j7b9lpjM4NX+Dj5TsBxgiwkhRJI"
        "iFEHs9HE6SRnjHYU6hrwOBBGGfKuNylAvs+mninLtf9sPiCke+Sk90usNMEzwApq"
        "cGrMxv/T2OT71pqZcE4Sg8hQ2MWNHldTzZWHuDxMNGy5pYE3IT7BCDTGat/iu1xQ"
        "Go7y7K3Rtnej3xpt64br8HIsT1Aw4g+QGN1bb8U+6iT9kre1tAJf6umW0+SP1MZQ"
        "2C261+r5NmOWmFEvJiU9LvaEfIUY6FZcyaVJXG//V83nMjiCxUp9tHCrLa+P/Sv3"
        "lPp8aS2ef71TLuzB14gOLKCzIWEovii0qfHRUfrJeAiwvZi3tDphKprIZYEr/qxv"
        "R0YCd4QLUqOwh/kWynztwPdo6ivRnqIRVfhLSgTEAArSrgWHFU1WC8Ckd6T5MpqJ"
        "hN0x6x8qBePZGHAdYwz8qa9h7wiNLFWBrLRj5DmQLl1CVxnpVrjW33MFso4P8n06"
        "0N4ghdKSSZsZozkNQ5b7O6yajYy+rSp6QpD8msb8oEX5imFKRaOcviQ2D4TRT45H"
        "JxKs63Tb9FtT1JoORzfkdv/E1bL3zSR6oYbTt2Stnpz+7kVqc8KR2N45EkFKxDkR"
        "w3IXOte0cq81xoU87S/ntf4KiVZaszuqb2XN2SgxnXBl4EDnpehPmqkD92SAlLrQ"
        "cTaxaSe47G28K+8MwoVt4eeVkj4UEsSfJN7rbCH2yKl2XJx5huDaS0xn2ODQyNRm"
        "gk+5I9hXMUiZDNLvEzx4zuyrcu2d0oXFo3ZoUtVFNCB//TQCf2x27ej9GjLXLDAE"
        "i7qnl9Xfb94n0IfeVyGte3+j6NP3DWv8OrLiUjNTaLv6Fay1yzfUaU6LI86+Jd6c"
        "kloiGhg7kE0/hd+ZKakZxU1vh0Vzc6DW7MFAPky75iCZlDXoBpZjTNGo5HR+mCW/"
        "ozblu60U9zZA8bn+voANuu/hYwxh+uY1sHTFZOqp2xicnnMChz/GTm1Je8XCkICY"
        "egeiHUryEHA6T6B/L9gW8S/R4ptMD0Sv6b1KHqqKeubwKltCWPUsr2En9iYypnz0"
        "6DEL5Wp8KMhrLid2AMPpLI0j1CWGJExXHpBWjfIC8vbYH4YKVl+euRo8eDcuKosb"
        "5hxUGM9Jvy1siVXUpIKpkZt2YLP5pEBP/EVOoHPh5LJomrLMpORr1wBKbEkfom7n"
        "pX1g817bK4IeYmZELI8zXUUtUkx3LgNTckwjx90Vt6oVXpFEICIUDF/LAVMUftzz"
        "6JUvbwOZo8iAZqcnVslAmRXeY/ZPp5eEHFfHlsb8VQ73Rd/p8XlFf5R1WuWiUGp2"
        "TzJ+VQvj3BTdQfOwSxR9RUk4xjqNabLqTFcQ7As246bHJXH6XVnd4DbEIDPfNa8F"
        "aWb/DNEgQAiXGqa6n7l7aFq5/6Kp0XeBBM0sOzJt4fy8JC6U0DEcMnWxKFDtMM7q"
        "06LubQYFCEEdQ5b1Qh2LbQZ898tegmeF++EZ4F4hvYebZPV8sM0ZcsKBXyCr585q"
        "s00PRxr0S6rReekGRBIvXzMojmid3dxc6DPpdV3x5zxlxaIBxO3i/6axknSSdxnS"
        "04/bemWqQ3CLf6mpSqfTIQJT1407GB4QINAAC9Ch3AXUR/n1jr64TGWzbIr8uDcn"
        "oVCJlOgmlXpmOwubigAzJattbWRi7k4QYBnA3/4QMjt73n2Co4+F/Qh4boYLpmwW"
        "G2SwcIw2PeXGr2LY2zwkPR4bcSyx1Z6UK5trQpWlpQCxgsvV/RvGzpN22RtHoihP"
        "H74K0cBIzCz7tK+jqeuWl1A7af7KmQ66fpRBr5ykTLOsa17WblkcIB/jDvqKfEcd"
        "xhPWJUwmOo4TIQS+xH8arLOy/NQFG2m14/yxwUemXC+QxLUYi6/FIcqwPBKjCdpQ"
        "tadRdyftQSKO0SP+GxUvamMZzWI780rXuOBkq5kyYLy9QF9bf/+bL6QLpe1WMCQl"
        "OeXZaCPoncgYoT0WZ17jB52Xb2lPWsyXYK54npszkbKJ4OIqfvF8xqRXcVe22VwJ"
        "uqT9Uy4+4KKQgQ7TXla7Gdm2H7mKl8YXQlsGCT2Ypc8O4t0Sfw7qYAuaDGf752Hb"
        "m3fl1bupcB2huIPlIaDP6IRR9XvTYIW2flbwYfhKLmoVKnG85uUi2qtqCjPOIuU3"
        "+peT0othfmwKQXaoOqO+V4r6wPL1VHxVFtIYmEdVt0RccUOvpOVR/OAHG9uHOzTm"
        "ueK5557Qxp0ojtZCHyN+hgoMZJLrvdKkTCxPNo2+mZQbHoVh2FnThZ9JbO49dB8l"
        "KXP4/MU5xAnjXMgKXtbfI8w6ZWATE/XWgf2VQMUpGp4wpy44yWQTxHxh/4T9540B"
        "GwG0FU0bkgrwA/erseGZnepqdmz5/ScCs84O5Xr5MbYhJLCGGxY6O5GqS+ooB2w0"
        "Mt87KbbE4bpYje9CAHH8FX3pDrJyLsyasA3zxmk4OmGpG7Z70ofONJtHRe56R528"
        "7vFmuazEEutXn81kNzB+3aJT1ga3vnWZw4CSvFKoWYSA7auLgrHSHFZdITfOrgtm"
        "QmGbFhM9kSBdY1UCnpzf65oos3PZWRa2twfUxxLAnPNtrxpRGyvtsapw7ljUagZm"
        "uyh3hLCjhAxYmnoE1dbyIWvpCqSlEtVjL1yb/nuLEzgvmZuV02fHxGuWgHTOMVGX"
        "pf81Rce3eoBK3lapW1wkzezlk3tcA2bZOtA9qbxdsbVR37kemzQ9K1e3Y0OWhtSj"
    >>).

ec_priv_int() -> 16#0707070707070707070707070707070707070707070707070707070707070707.

unsigned(N) -> list_to_binary(lists:reverse(bytes(N))).

bytes(0) -> [];
bytes(N) -> [N band 255 | bytes(N bsr 8)].
