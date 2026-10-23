-module(crypto_sign_probe).
-export([main/0, host/0, unsupported_sign/0, unsupported_heavy/0, unsupported_verify/0, unsupported_curve/0]).

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
                                                               password => "p"}) end]].

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
unsupported_sign() ->
    crypto:sign(mldsa44, none, <<"abc">>, <<1>>).

unsupported_heavy() ->
    crypto:sign(slh_dsa_sha2_128s, none, <<"abc">>, <<1>>).

unsupported_verify() ->
    crypto:verify(mldsa44, none, <<"abc">>, <<1>>, <<1>>).

unsupported_curve() ->
    crypto:sign(ecdsa, sha256, <<"abc">>, [<<1>>, sect163k1]).

ec_priv_int() -> 16#0707070707070707070707070707070707070707070707070707070707070707.

unsigned(N) -> list_to_binary(lists:reverse(bytes(N))).

bytes(0) -> [];
bytes(N) -> [N band 255 | bytes(N bsr 8)].
