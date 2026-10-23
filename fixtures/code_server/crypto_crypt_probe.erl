-module(crypto_crypt_probe).
-export([main/0, host/0]).

%% Write the results of crypto:public_encrypt/4, private_decrypt/4,
%% private_encrypt/4, and public_decrypt/4, which call pkey_crypt_nif/6
%% (crypto.erl:3476-3601, pkey.c:1173-1581). RSA PKCS #1 v1.5 and OAEP
%% encryption add random bytes, so the probe writes the size of each cipher
%% text and the result of its decryption. A private encryption with PKCS #1
%% v1.5, X9.31, or no padding, and a public encryption with no padding do
%% not add random bytes, so the probe writes their bytes. An error writes
%% its class and reason, which names the C file and line of the failed
%% check (crypto.erl:761-775, common.c:29-90).
main() ->
    Msg = <<"abc">>,
    Pkcs1 = [crypto:private_encrypt(rsa, Msg, Key, Opts)
             || Key <- [rsa_priv(), rsa_priv3()],
                Opts <- [[], rsa_pkcs1_padding, [{rsa_pad, rsa_pkcs1_padding}]]],
    Recovered = [crypto:public_decrypt(rsa, C, rsa_pub(), []) || C <- Pkcs1],
    Block = binary:copy(<<1>>, 128),
    NoPad = [crypto:private_encrypt(rsa, Block, rsa_priv(), rsa_no_padding),
             crypto:public_encrypt(rsa, Block, rsa_pub(), rsa_no_padding)],
    NoPadBack = [crypto:public_decrypt(rsa, hd(NoPad), rsa_pub(), rsa_no_padding),
                 crypto:private_decrypt(rsa, lists:last(NoPad), rsa_priv(), rsa_no_padding)],
    Digest = crypto:hash(sha256, Msg),
    Md = crypto:private_encrypt(rsa, Digest, rsa_priv(), [{signature_md, sha256}]),
    MdBack = [crypto:public_decrypt(rsa, Md, rsa_pub(), [{signature_md, sha256}]),
              crypto:public_decrypt(rsa, Md, rsa_pub(), [])],
    Random = [round_trip(Msg, Opts)
              || Opts <- [[], rsa_pkcs1_padding, rsa_pkcs1_oaep_padding,
                          [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_md, sha256}],
                          [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_md, sha256},
                           {rsa_mgf1_md, sha512}],
                          [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_label, <<"l">>}],
                          [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_label, <<>>}],
                          [{rsa_oaep_md, sha256}, {rsa_mgf1_md, sha}, {rsa_oaep_label, <<"l">>}]]],
    Oaep = crypto:public_encrypt(rsa, Msg, rsa_pub(),
                                 [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_label, <<"l">>}]),
    Mismatch = [bad_reason(fun() -> crypto:private_decrypt(rsa, Oaep, rsa_priv(), Opts) end)
                || Opts <- [[{rsa_padding, rsa_pkcs1_oaep_padding}],
                            [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_label, <<"m">>}],
                            [{rsa_padding, rsa_pkcs1_oaep_padding}, {rsa_oaep_label, <<"l">>}]]],
    io:format("~p~n", [[Pkcs1, Recovered, NoPad, NoPadBack, Md, MdBack, Random,
                        Mismatch, errors()]]).

round_trip(Msg, Opts) ->
    C = crypto:public_encrypt(rsa, Msg, rsa_pub(), Opts),
    {byte_size(C), crypto:private_decrypt(rsa, C, rsa_priv(), Opts)}.

errors() ->
    Msg = <<"abc">>,
    [bad_reason(Fun)
     || Fun <- [fun() -> crypto:public_encrypt(nosuch, Msg, rsa_pub(), []) end,
                fun() -> crypto:public_encrypt("rsa", Msg, rsa_pub(), []) end,
                fun() -> crypto:public_encrypt(rsa, "abc", rsa_pub(), []) end,
                fun() -> crypto:private_encrypt(rsa, <<1:7>>, rsa_priv(), []) end,
                fun() -> crypto:public_encrypt(rsa, Msg, #{}, []) end,
                fun() -> crypto:private_decrypt(rsa, Msg, #{}, []) end,
                fun() -> crypto:public_encrypt(rsa, Msg, [<<1>>], []) end,
                fun() -> crypto:private_decrypt(rsa, Msg, [<<1>>], []) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), 1) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), nosuch) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), rsa_pkcs1_pss_padding) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), rsa_sslv23_padding) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [a]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{a, b, c}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{a, b}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{1, b}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{signature_md, 1}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{signature_md, nosuch}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{rsa_mgf1_md, 1}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{rsa_mgf1_md, nosuch}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{rsa_oaep_label, a}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{rsa_oaep_md, 1}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{rsa_oaep_md, nosuch}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), [{signature_md, sha}]) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), rsa_x931_padding) end,
                fun() -> crypto:private_encrypt(rsa, Msg, rsa_priv(), rsa_pkcs1_oaep_padding) end,
                fun() -> crypto:public_decrypt(rsa, Msg, rsa_pub(), []) end,
                fun() -> crypto:public_encrypt(rsa, binary:copy(Msg, 50), rsa_pub(), []) end,
                fun() -> crypto:public_encrypt(rsa, Msg, rsa_pub(), rsa_no_padding) end,
                fun() -> crypto:public_encrypt(ecdsa, Msg, [ec_priv(), secp256r1], [a]) end,
                fun() -> crypto:public_encrypt(ecdsa, Msg, [<<1>>, secp256r1], []) end,
                fun() -> crypto:private_encrypt(dss, Msg, [<<1>>], []) end,
                fun() -> crypto:private_encrypt(eddsa, Msg, [<<1>>, ed25519], []) end]].

%% Write the results that depend on the libcrypto of the host: X9.31
%% padding for a private encryption, the implicit rejection of a bad PKCS
%% #1 v1.5 cipher text (OpenSSL 3.2 and later), and the algorithms other
%% than RSA.
host() ->
    Msg = <<"abc">>,
    Ec = [ec_priv(), secp256r1],
    Funs = [fun() -> crypto:private_encrypt(rsa, Msg, rsa_priv(), rsa_x931_padding) end,
            fun() -> crypto:private_decrypt(rsa, Msg, rsa_priv(), []) end,
            fun() -> crypto:public_encrypt(ecdsa, Msg, [ec_pub(), secp256r1], []) end,
            fun() -> crypto:private_decrypt(ecdsa, Msg, Ec, []) end,
            fun() -> crypto:public_decrypt(ecdsa, Msg, [ec_pub(), secp256r1], []) end,
            fun() -> is_binary(crypto:private_encrypt(ecdsa, binary:copy(<<1>>, 32), Ec, [])) end,
            fun() -> crypto:private_encrypt(eddsa, Msg, [ed_priv(), ed25519], []) end,
            fun() -> crypto:public_encrypt(eddsa, Msg, [ed_pub(), ed25519], []) end,
            fun() -> crypto:public_encrypt(dss, Msg, dss_pub(), []) end,
            fun() -> crypto:public_decrypt(dss, Msg, dss_pub(), []) end,
            fun() -> is_binary(crypto:private_encrypt(dss, binary:copy(<<1>>, 20), dss_priv(), [])) end],
    io:format("~p~n", [[bad_reason(Fun) || Fun <- Funs]]).

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

unsigned(N) -> list_to_binary(lists:reverse(bytes(N))).

bytes(0) -> [];
bytes(N) -> [N band 255 | bytes(N bsr 8)].
