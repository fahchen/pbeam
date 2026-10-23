-module(crypto_probe).
-export([main/0, nifs/0, info/0, supports/0, fips/0, oneshot/0, oneshot_host/0, xof_sizes/0, ciphers/0, ciphers_host/0, cipher_order/0, contexts/0, mac_contexts/0, mac_contexts_host/0, cipher_contexts/0, mac_poly1305_final/0, mac_poly1305_update/0, unsupported/0]).

%% Write the results of crypto:strong_rand_bytes/1, one term on each line.
%% The first call loads crypto on demand, and its -on_load function loads
%% the NIF library (crypto.erl:4432-4510). The NIF strong_rand_bytes_nif/1
%% gives a binary of the size, <<>> for 0, and badarg for a term that is not
%% an unsigned C int or that is above INT_MAX (rand.c:26-50).
main() ->
    Sizes = [byte_size(crypto:strong_rand_bytes(Size)) || Size <- [0, 1, 16, 1024]],
    Binaries = [is_binary(crypto:strong_rand_bytes(Size)) || Size <- [1, 16]],
    Bad = [bad(fun() -> crypto:strong_rand_bytes(Size) end)
           || Size <- [-1, 2147483648, 1 bsl 64, 1.0, a, <<1>>]],
    io:format("~p~n", [[erlang:system_info(build_type), crypto:strong_rand_bytes(0),
                        Sizes, Binaries, Bad]]).

%% Write the results of the crypto NIFs that PBeam links: hash_nif/2,
%% pbkdf2_hmac_nif/5, strong_rand_range_nif/1, info_nif/0, and info_lib/0.
%% A random value prints only as its shape, because its bytes change from
%% run to run. The value of the range 1 is always <<>> (rand.c:54-85). The
%% errors print the class and the reason, which names the C file and line
%% of the failed check (crypto.erl:761-775, common.c:29-90). PBeam gives a
%% frame location of `[]` in each stack trace, so the probe does not compare
%% the location (row "Exception stack trace" of docs/erts-differences.md).
nifs() ->
    Digests = [sha, sha224, sha256, sha384, sha512, sha512_224, sha512_256],
    Hashes = [{Digest, crypto:hash(Digest, <<"abc">>)} || Digest <- Digests],
    HashErrors = [bad_reason(fun() -> crypto:hash(Type, <<"abc">>) end) ||
                     Type <- [nosuch, "sha", <<"sha">>]],
    Pbkdf2 = [{Digest, crypto:pbkdf2_hmac(Digest, <<"password">>, <<"salt">>, 1, 20)}
              || Digest <- Digests],
    Pbkdf2Errors = [bad_reason(fun() -> crypto:pbkdf2_hmac(Type, Pass, Salt, Iter, KeyLen) end) ||
                       {Type, Pass, Salt, Iter, KeyLen} <-
                           [{sha3_256, <<"p">>, <<"s">>, 1, 16},
                            {nosuch, <<"p">>, <<"s">>, 1, 16},
                            {sha, <<"p">>, <<"s">>, 0, 16},
                            {sha, <<"p">>, <<"s">>, -1, 16},
                            {sha, <<"p">>, <<"s">>, 1.0, 16},
                            {sha, <<"p">>, <<"s">>, 1 bsl 32, 16},
                            {sha, <<"p">>, <<"s">>, 1 bsl 31, 16},
                            {sha, <<"p">>, <<"s">>, 1, 0},
                            {sha, "p", <<"s">>, 1, 16},
                            {sha, <<"p">>, "s", 1, 16}]],
    %% The value 2^32 + 1 keeps the low 32 bits of the C int (pbkdf2_hmac.c:76-96).
    Pbkdf2Wrapped = crypto:pbkdf2_hmac(sha, <<"p">>, <<"s">>, (1 bsl 32) + 1, 16)
        =:= crypto:pbkdf2_hmac(sha, <<"p">>, <<"s">>, 1, 16),
    Ranges = [{is_integer(crypto:strong_rand_range(N)), crypto:strong_rand_range(N) < N}
              || N <- [2, 10, 1024]],
    RangeOne = crypto:strong_rand_range(1),
    BinaryRanges = [{Value, is_binary(Value),
                     crypto:bytes_to_integer(Value) < crypto:bytes_to_integer(Range)}
                    || Range <- [<<1>>, <<0, 1>>],
                       Value <- [crypto:strong_rand_range(Range)]],
    BinaryShapes = [{is_binary(Value),
                     byte_size(Value) =< byte_size(Range),
                     crypto:bytes_to_integer(Value) < crypto:bytes_to_integer(Range)}
                    || Range <- [<<10>>, <<1, 0>>, <<255, 255>>],
                       Value <- [crypto:strong_rand_range(Range)]],
    RangeErrors = [bad_reason(fun() -> crypto:strong_rand_range(Range) end) ||
                      Range <- [0, -1, 1.0, a, <<0>>, <<>>, <<0, 0>>]],
    io:format("~p~n", [[Hashes, HashErrors, Pbkdf2, Pbkdf2Errors, Pbkdf2Wrapped,
                        Ranges, RangeOne, BinaryRanges, BinaryShapes, RangeErrors]]).

%% Write the fields of crypto:info/0 and crypto:info_lib/0 that do not
%% depend on the libcrypto build of the host. The pinned ERTS links a
%% different OpenSSL on macOS and Linux, so the version fields and the link
%% type stay out of the comparison.
info() ->
    Fields = crypto:info(),
    [{Name, Number, Text}] = crypto:info_lib(),
    io:format("~p~n", [[lists:sort(maps:keys(Fields)),
                        maps:get(compile_type, Fields),
                        maps:get(fips_provider_available, Fields),
                        maps:get(otp_crypto_version, Fields),
                        {Name, is_integer(Number), is_binary(Text)}]]).

%% Write the count of the items of crypto:supports/0, the items that PBeam
%% can run, and whether the list has only those items. The NIFs of the lists
%% are in algorithms.c:86-224 and digest.c:226-242; supports/0 builds the
%% list (crypto.erl:843-895). The pinned ERTS lists each algorithm of its
%% OpenSSL. PBeam lists only the algorithms that it can run: the digests of
%% hash_nif/2 and the MACs of mac_nif/4.
supports() ->
    Run = [blake2s, blake2b, sm3, shake256, shake128, sha3_512, sha3_384, sha3_256,
           sha3_224, sha512_256, sha512_224, sha512, sha384, sha256, sha224, sha,
           ripemd160, md5, cmac, hmac, poly1305] ++ run_ciphers() ++
          [sm4_gcm, sm4_ccm, aes_cbc, aes_ccm, aes_cfb128, aes_cfb8, aes_ctr, aes_ecb, aes_gcm],
    All = crypto:supports(),
    Items = [{Type, [Item || Item <- List, lists:member(Item, Run)]} || {Type, List} <- All],
    Lists = [crypto:supports(Type) || {Type, _} <- All],
    %% The probe cipher_order/0 checks the order of the ciphers, so this probe
    %% sorts them. The libcrypto of the Linux CI host has no sm4_gcm and
    %% sm4_ccm, so the list leaves them out.
    Sorted = [{Type, case Type of ciphers -> lists:sort(List) -- [sm4_gcm, sm4_ccm]; _ -> List end}
              || {Type, List} <- Items],
    io:format("~p~n", [[length(lists:append([List || {_, List} <- All])), Sorted,
                        All =:= Items, Lists =:= [List || {_, List} <- All]]]).

%% Write the FIPS status and the results of crypto:enable_fips_mode/1. A
%% build without FIPS support gives not_supported, false for true, true
%% for false, and badarg for each other term (fips.c:27-62). The calls of
%% the deprecated enable_fips_mode/1 go through apply/3 (crypto.erl:183).
fips() ->
    Enable = fun(Mode) -> apply(crypto, enable_fips_mode, [Mode]) end,
    io:format("~p~n", [[crypto:info_fips(), Enable(true), Enable(false),
                        bad_reason(fun() -> Enable(yes) end)]]).

%% Write the results of hash_info/1, hash_nif/2 for each digest, mac_nif/4,
%% and hash_equals_nif/2 (hash.c:74-164, mac.c:188-505,
%% hash_equals.c:27-53). The digest table is digest.c:25-179, and the
%% cipher table of cmac is cipher.c:30-173. A failed MAC computation gives
%% the error of mac.c:405.
oneshot() ->
    Digests = [md5, ripemd160, sha, sha224, sha256, sha384, sha512, sha512_224,
               sha512_256, sha3_224, sha3_256, sha3_384, sha3_512, shake128, shake256,
               sm3, blake2b, blake2s],
    %% The size of a SHAKE digest depends on the libcrypto of the host, so
    %% the probe xof_sizes/0 prints it apart.
    Infos = [bad_reason(fun() -> maps:remove(size, crypto:hash_info(Type)) end)
             || Type <- Digests ++ [nosuch, 1]],
    Hashes = [bad_reason(fun() -> crypto:hash(Type, <<"abc">>) end) || Type <- Digests],
    Hmacs = [bad_reason(fun() -> crypto:mac(hmac, Type, <<"key">>, [<<"te">>, $x, "t"]) end)
             || Type <- Digests ++ [nosuch, 1]],
    Ciphers = [{des_ede3_cbc, 24}, {sm4_cbc, 16}, {aes_128_cbc, 16}, {aes_192_cbc, 24},
               {aes_256_cbc, 32}, {aes_cbc, 24}, {aes_128_cbc, 8}, {sm4_cbc, 8},
               {nosuch, 16}, {"aes_128_cbc", 16}],
    Cmacs = cmacs(Ciphers),
    Poly = [bad_reason(fun() -> crypto:mac(poly1305, Key, <<"text">>) end)
            || Key <- [binary:copy(<<3>>, 32), binary:copy(<<3>>, 31), <<>>]],
    MacErrors = [bad_reason(fun() -> crypto:mac(Type, Sub, Key, Text) end)
                 || {Type, Sub, Key, Text} <-
                        [{hmac, sha, <<"k">>, a}, {hmac, sha, a, <<"t">>},
                         {hmac, sha, [<<"k">>], [<<"t">>]}, {hmac, sha, <<>>, <<>>},
                         {nosuch, sha, <<"k">>, <<"t">>}, {1, sha, <<"k">>, a},
                         {poly1305, undefined, [<<0:256>>], <<"t">>}]],
    Equals = [bad_reason(fun() -> crypto:hash_equals(A, B) end)
              || {A, B} <- [{<<"ab">>, <<"ab">>}, {<<"ab">>, <<"ac">>}, {<<>>, <<>>},
                            {<<"a">>, <<"ab">>}, {"ab", <<"ab">>}, {<<"ab">>, a},
                            {<<1:7>>, <<1:7>>}]],
    io:format("~p~n", [[Infos, Hashes, Hmacs, Cmacs, Poly, MacErrors, Equals]]).

%% Write the results that depend on the libcrypto of the host: md4, and
%% cmac with a cipher that is not CBC or that the OpenSSL 3 default
%% provider does not have. The OpenSSL 3.5.8 of the pinned macOS ERTS has
%% no md4 (digest.c:186-188), cannot fetch rc2, rc4, des, or blowfish
%% (mac.c:344-347), and computes cmac only with a CBC cipher (mac.c:400-405).
oneshot_host() ->
    Md4 = [bad_reason(Fun) || Fun <- [fun() -> crypto:hash_info(md4) end,
                                       fun() -> crypto:hash(md4, <<"abc">>) end,
                                       fun() -> crypto:hash_init(md4) end,
                                       fun() -> crypto:mac(hmac, md4, <<"key">>, <<"text">>) end]],
    Ciphers = [{rc2_cbc, 16}, {rc4, 16}, {des_cbc, 8}, {des_cfb, 8}, {des_ecb, 8},
               {des_ede3_cbc, 16}, {des_ede3_cfb, 24}, {blowfish_cbc, 16},
               {blowfish_cfb64, 16}, {blowfish_ofb64, 16}, {blowfish_ecb, 16},
               {sm4_ecb, 16}, {sm4_cfb, 16}, {sm4_ofb, 16}, {sm4_ctr, 16}, {sm4_gcm, 16},
               {sm4_ccm, 16}, {aes_128_ofb, 16}, {aes_128_cfb8, 16}, {aes_128_cfb128, 16},
               {aes_128_ecb, 16}, {aes_128_ctr, 16}, {chacha20, 32},
               {chacha20_poly1305, 32}, {aes_128_gcm, 16}, {aes_256_ccm, 32},
               {aes_ecb, 16}],
    io:format("~p~n", [[Md4, cmacs(Ciphers)]]).

cmacs(Ciphers) ->
    [{Cipher, bad_reason(fun() ->
                                 crypto:mac(cmac, Cipher, binary:copy(<<7>>, Size), <<"text">>)
                         end)}
     || {Cipher, Size} <- Ciphers].

%% Write the sizes of hash_info/1 for the SHAKE digests. A fetched SHAKE md
%% gives 16/32 on OpenSSL 3.0-3.3 and 0 from 3.4 (evp_lib.c:1041 in 3.5.5).
xof_sizes() ->
    io:format("~p~n", [[maps:get(size, crypto:hash_info(Type)) || Type <- [shake128, shake256]]]).

%% Write the results of the hash context NIFs (hash.c:168-273, 561-589).
%% An update gives a new context and leaves the old one as it was, and a
%% final call reads a copy, so each context stays usable. A SHAKE context
%% gives the default xoflen of 16 or 32 bytes (hash.c:194-201), and
%% hash_final_xof/2 takes the length in bits. crypto:hash/2 sends an input
%% of more than 20000 bytes through the contexts (crypto.erl:4535-4540).
contexts() ->
    S0 = crypto:hash_init(sha256),
    S1 = crypto:hash_update(S0, <<"abc">>),
    S2 = crypto:hash_update(S1, ["d", <<"e">>]),
    Finals = [crypto:hash_final(S) || S <- [S0, S1, S1, S2]],
    Digests = [crypto:hash_final(crypto:hash_update(crypto:hash_init(Type), <<"abc">>))
               || Type <- [md5, ripemd160, sha, sha512, sha3_256, shake128, shake256,
                           sm3, blake2b, blake2s]],
    X0 = crypto:hash_init(shake256),
    X1 = crypto:hash_update(X0, <<"abc">>),
    Xofs = [crypto:hash_final_xof(X, Length) || X <- [X0, X1], Length <- [0, 7, 8, 100, 256]],
    Large = crypto:hash(sha, binary:copy(<<1>>, 20001))
        =:= crypto:hash_final(crypto:hash_update(crypto:hash_init(sha),
                                                 binary:copy(<<1>>, 20001))),
    Errors = [bad_reason(Fun)
              || Fun <- [fun() -> crypto:hash_init(nosuch) end,
                         fun() -> crypto:hash_init("sha") end,
                         fun() -> crypto:hash_update(make_ref(), <<"a">>) end,
                         fun() -> crypto:hash_update(S0, a) end,
                         fun() -> crypto:hash_final(make_ref()) end,
                         fun() -> crypto:hash_final(S0 =/= x) end,
                         fun() -> crypto:hash_final_xof(make_ref(), 8) end,
                         fun() -> crypto:hash_final_xof(X0, -1) end,
                         fun() -> crypto:hash_final_xof(X0, 1 bsl 32) end,
                         fun() -> crypto:hash_final_xof(X0, 8.0) end,
                         fun() -> crypto:hash_final_xof(S0, 64) end]],
    io:format("~p~n", [[is_reference(S0), Finals, Digests, Xofs, Large, Errors]]).

%% Write the results of the MAC context NIFs (mac.c:564-887). A context
%% changes in place: mac_update/2 gives back its argument. After a final, an
%% CMAC context goes on from its moved CBC chain, and a Poly1305 context
%% keeps a short update.
mac_contexts() ->
    H = crypto:mac_init(hmac, sha256, <<"key">>),
    Same = [H =:= crypto:mac_update(H, <<"abc">>), H =:= crypto:mac_update(H, ["d", <<"e">>])],
    H1 = crypto:mac_final(H),
    Hmacs = [crypto:mac_final(crypto:mac_update(crypto:mac_init(hmac, Type, Key), <<"abc">>))
             || Type <- [md5, ripemd160, sha, sha512, sha3_256, sm3, blake2b, blake2s],
                Key <- [<<>>, <<"k">>, binary:copy(<<7>>, 200)]],
    Cmacs = [cmac(Cipher, Key, Parts)
             || {Cipher, Key} <- [{aes_128_cbc, binary:copy(<<1>>, 16)},
                                  {aes_cbc, binary:copy(<<2>>, 32)},
                                  {des_ede3_cbc, binary:copy(<<3>>, 24)}],
                Parts <- [[], [<<"abc">>], [binary:copy(<<4>>, 16)],
                          [<<"a">>, binary:copy(<<5>>, 15), <<"b">>],
                          [binary:copy(<<6>>, 33), <<>>, binary:copy(<<7>>, 40)]]],
    P = crypto:mac_init(poly1305, binary:copy(<<8>>, 32)),
    P = crypto:mac_update(P, binary:copy(<<9>>, 20)),
    P1 = crypto:mac_final(P),
    PAfter = [P =:= crypto:mac_update(P, <<1, 2, 3>>), P =:= crypto:mac_update(P, <<>>)],
    Short = crypto:mac_finalN(crypto:mac_update(crypto:mac_init(hmac, sha, <<"k">>), <<"a">>), 4),
    Large = crypto:mac(hmac, sha, <<"k">>, binary:copy(<<1>>, 70000))
        =:= crypto:mac_final(crypto:mac_update(crypto:mac_init(hmac, sha, <<"k">>),
                                               binary:copy(<<1>>, 70000))),
    Errors = [bad_reason(Fun)
              || Fun <- [fun() -> crypto:mac_init(nosuch, sha, <<"k">>) end,
                         fun() -> crypto:mac_init(poly1305, <<"k">>) end,
                         fun() -> crypto:mac_init(hmac, nosuch, <<"k">>) end,
                         fun() -> crypto:mac_init(hmac, shake128, <<"k">>) end,
                         fun() -> crypto:mac_init(cmac, nosuch, <<"k">>) end,
                         fun() -> crypto:mac_init(cmac, chacha20, <<"k">>) end,
                         fun() -> crypto:mac_init(cmac, des_ede3_cbc, binary:copy(<<1>>, 16)) end,
                         fun() -> crypto:mac_update(make_ref(), a) end,
                         fun() -> crypto:mac_update(make_ref(), <<"a">>) end,
                         fun() -> crypto:mac_final(make_ref()) end,
                         fun() -> crypto:mac_final(nosuch) end,
                         fun() -> crypto:mac_final(crypto:hash_init(sha)) end,
                         fun() -> crypto:hash_update(P, <<"a">>) end]],
    io:format("~p~n", [[is_reference(H), Same, H1, Hmacs, Cmacs, P1, PAfter,
                        Short, Large, Errors]]).

%% Write the results of the MAC context NIFs that depend on the libcrypto
%% of the host. With the OpenSSL 3.5.8 of the pinned macOS ERTS, an HMAC
%% context fails after its final (mac.c:840, 879), md4 and a cipher that is
%% not CBC fail EVP_MAC_init (mac.c:750), and des_cbc has no cipher
%% (mac.c:688).
mac_contexts_host() ->
    H = crypto:mac_init(hmac, sha256, <<"key">>),
    crypto:mac_final(H),
    Ended = [bad_reason(Fun)
             || Fun <- [fun() -> crypto:mac_update(H, <<"f">>) end,
                        fun() -> crypto:mac_final(H) end,
                        fun() -> crypto:mac_finalN(H, 3) end]],
    Empty = H =:= crypto:mac_update(H, <<>>),
    Inits = [bad_reason(fun() -> crypto:mac_init(Type, Sub, binary:copy(<<1>>, Size)) end)
             || {Type, Sub, Size} <- [{hmac, md4, 1}, {cmac, des_cbc, 8},
                                      {cmac, aes_128_ecb, 16}, {cmac, aes_128_gcm, 16}]],
    io:format("~p~n", [[Ended, Empty, Inits]]).

%% Give the CMAC of the parts, a second final, and the final after one more
%% update. CMAC_Final pads the last block in place and moves the CBC chain
%% (OpenSSL cmac.c:247-276), so the three values differ.
cmac(Cipher, Key, Parts) ->
    C = crypto:mac_init(cmac, Cipher, Key),
    [C = crypto:mac_update(C, Part) || Part <- Parts],
    F1 = crypto:mac_final(C),
    F2 = crypto:mac_final(C),
    C = crypto:mac_update(C, <<"tail">>),
    [F1, F2, crypto:mac_final(C)].

%% Use a Poly1305 context after its final in a way that stops the pinned
%% ERTS: Poly1305_Final clears the context (OpenSSL poly1305.c), so a second
%% final, or an update that fills a block, calls a cleared function pointer.
mac_poly1305_final() ->
    crypto:mac_final(ended_poly1305()).

mac_poly1305_update() ->
    crypto:mac_update(ended_poly1305(), binary:copy(<<1>>, 16)).

ended_poly1305() ->
    P = crypto:mac_init(poly1305, binary:copy(<<8>>, 32)),
    crypto:mac_final(P),
    P.

%% Write crypto:supports(ciphers) in its order. cipher_algorithms/0 sorts the
%% table by atom (cipher.c:239, 395-416), and the atom indexes come from the
%% atom chunks of the modules that load first, so the order is the same on
%% each run. The probe keeps only the ciphers that PBeam runs, because the
%% libcrypto of the Linux CI host also has the legacy ciphers.
%% Write the results of the cipher context NIFs ng_crypto_init_nif/4,
%% ng_crypto_update_nif/2, ng_crypto_final_nif/1, ng_crypto_get_data_nif/1
%% (api_ng.c:91-892), aead_cipher_init_nif/4, and aead_cipher_nif/4
%% (aead.c:69-377), with the error tuple of each failed check. A context
%% changes in place. Random padding prints only its size.
cipher_contexts() ->
    Key = binary:copy(<<1>>, 16),
    Iv = binary:copy(<<2>>, 16),
    Data = <<"The quick brown fox jumps over th">>,
    Stream = fun(Cipher, K, I, Opts, Parts) ->
                 bad_reason(fun() ->
                     S = crypto:crypto_init(Cipher, K, I, Opts),
                     D0 = crypto:crypto_get_data(S),
                     Outs = [crypto:crypto_update(S, P) || P <- Parts],
                     D1 = crypto:crypto_get_data(S),
                     F = bad_reason(fun() -> crypto:crypto_final(S) end),
                     D2 = crypto:crypto_get_data(S),
                     F2 = bad_reason(fun() -> crypto:crypto_final(S) end),
                     {D0, Outs, D1, F, D2, F2}
                 end)
             end,
    Parts = [<<"The quick ">>, [<<"brown">>, $\s], <<>>, <<"fox jumps over th">>],
    Pads = [Stream(C, binary:copy(<<1>>, KL), binary:copy(<<2>>, IL), [{padding, P}], Parts)
            || {C, KL, IL} <- [{aes_128_cbc, 16, 16}, {aes_256_ecb, 32, 0}, {aes_128_ctr, 16, 16},
                               {des_ede3_cbc, 24, 8}, {aes_192_cfb8, 24, 16}, {chacha20, 32, 16},
                               {aes_cbc, 16, 16}],
               P <- [undefined, none, zero, pkcs_padding]],
    Random = bad_reason(fun() ->
                 S = crypto:crypto_init(aes_128_cbc, Key, Iv, [{padding, random}]),
                 <<_/binary>> = crypto:crypto_update(S, Data),
                 {byte_size(crypto:crypto_final(S)), crypto:crypto_get_data(S)}
             end),
    Opts = [Stream(aes_128_cbc, Key, Iv, O, [Data])
            || O <- [true, false, undefined, [], [{encrypt, false}], [{encrypt, undefined}],
                     [{encrypt, false}, {padding, pkcs_padding}], [{encrypt, true}, {encrypt, true}],
                     [{padding, bad}], [{bad, 1}], [x], 1]],
    Enc = crypto:crypto_one_time(aes_128_cbc, Key, Iv, Data, [{padding, pkcs_padding}]),
    Dec = [Stream(aes_128_cbc, Key, Iv, [{encrypt, false}, {padding, P}],
                  [binary:part(Enc, 0, 20), binary:part(Enc, 20, 28)])
           || P <- [pkcs_padding, none, undefined, zero]],
    NoIv = bad_reason(fun() ->
               S = crypto:crypto_init(aes_128_ecb, Key, true),
               {crypto:crypto_update(S, binary:copy(<<7>>, 16)), crypto:crypto_get_data(S)}
           end),
    Reinit = bad_reason(fun() ->
                 S = crypto:crypto_init(aes_128_ctr, Key, Iv, true),
                 A = crypto:crypto_update(S, <<"abc">>),
                 S = crypto:crypto_init(S, <<>>, <<>>, false),
                 B = crypto:crypto_update(S, <<"def">>),
                 D = crypto:crypto_get_data(S),
                 E = bad_reason(fun() -> crypto:crypto_init(S, <<>>, <<>>, perhaps) end),
                 {A, B, D, E}
             end),
    Mac = crypto:mac_init(hmac, sha256, <<"k">>),
    InitErrors = [bad_reason(fun() -> crypto:crypto_init(C, K, I, true) end)
                  || {C, K, I} <- [{aes_128_cbc, <<1>>, Iv}, {aes_128_cbc, a, Iv},
                                   {nosuch, Key, Iv}, {"aes", Key, Iv}, {Mac, Key, Iv},
                                   {make_ref(), Key, Iv}, {aes_128_gcm, Key, Iv},
                                   {aes_128_cbc, Key, <<1>>}, {aes_128_cbc, Key, a},
                                   {aes_128_cbc, Key, undefined}]],
    S0 = crypto:crypto_init(aes_128_cbc, Key, Iv, true),
    StateErrors = [bad_reason(F)
                   || F <- [fun() -> crypto:crypto_update(S0, a) end,
                            fun() -> crypto:crypto_update(Mac, <<"a">>) end,
                            fun() -> crypto:crypto_update(make_ref(), a) end,
                            fun() -> crypto:crypto_final(Mac) end,
                            fun() -> crypto:crypto_get_data(Mac) end,
                            fun() -> crypto:crypto_get_data(a) end]],
    io:format("~p~n", [[Pads, Random, Opts, Dec, NoIv, Reinit, InitErrors, StateErrors,
                        aead_contexts()]]).

aead_contexts() ->
    Run = fun(Cipher, KL, IL) ->
              Key = binary:copy(<<5>>, KL),
              Iv = binary:copy(<<6>>, IL),
              E = crypto:crypto_one_time_aead_init(Cipher, Key, 16, true),
              Out1 = bad_reason(fun() -> crypto:crypto_one_time_aead(E, Iv, <<"plain text">>, <<"aad">>) end),
              Out2 = bad_reason(fun() -> crypto:crypto_one_time_aead(E, Iv, [<<"plain">>], <<"aad">>) end),
              D = crypto:crypto_one_time_aead_init(Cipher, Key, 16, false),
              Back = case Out1 of
                         <<_/binary>> ->
                             [bad_reason(fun() -> crypto:crypto_one_time_aead(D, Iv, T, A) end)
                              || {T, A} <- [{Out1, <<"aad">>}, {Out1, <<"bad">>},
                                            {binary:part(Out1, 0, 9), <<"aad">>}]];
                         _ -> []
                     end,
              Short = crypto:crypto_one_time_aead_init(Cipher, Key, 8, true),
              S = bad_reason(fun() -> crypto:crypto_one_time_aead(Short, Iv, <<"text">>, <<>>) end),
              Zero = crypto:crypto_one_time_aead_init(Cipher, Key, 0, true),
              Z = bad_reason(fun() -> crypto:crypto_one_time_aead(Zero, Iv, <<"text">>, <<>>) end),
              {Cipher, Out1, Out2, Back, S, Z}
          end,
    Ciphers = [Run(C, KL, IL) || {C, KL, IL} <- [{aes_128_gcm, 16, 12}, {aes_256_gcm, 32, 12},
                                                 {aes_128_ccm, 16, 12}, {aes_192_ccm, 24, 7},
                                                 {chacha20_poly1305, 32, 12}, {aes_gcm, 16, 12}]],
    Key = binary:copy(<<1>>, 16),
    InitErrors = [case bad_reason(fun() -> crypto:crypto_one_time_aead_init(C, K, L, E) end) of
                      R when is_reference(R) -> state;
                      R -> R
                  end
                  || {C, K, L, E} <- [{aes_128_gcm, Key, 16, perhaps}, {"gcm", Key, 16, true},
                                      {aes_128_gcm, Key, -1, true}, {aes_128_gcm, Key, 1 bsl 32, true},
                                      {aes_128_gcm, Key, 1 bsl 31, true}, {aes_128_gcm, <<1>>, 16, true},
                                      {nosuch, Key, 16, true}, {aes_128_cbc, Key, 16, true},
                                      {aes_128_gcm, [Key], 16, false}]],
    E = crypto:crypto_one_time_aead_init(aes_128_gcm, Key, 16, true),
    D = crypto:crypto_one_time_aead_init(aes_128_gcm, Key, 16, false),
    C = crypto:crypto_one_time_aead_init(aes_128_ccm, Key, 3, true),
    Iv = binary:copy(<<2>>, 12),
    Errors = [bad_reason(F)
              || F <- [fun() -> crypto:crypto_one_time_aead(make_ref(), Iv, <<>>, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(a, Iv, <<>>, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(E, a, <<>>, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(E, Iv, a, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(E, Iv, <<>>, a) end,
                       fun() -> crypto:crypto_one_time_aead(E, <<>>, <<"t">>, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(D, Iv, <<"short">>, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(C, Iv, <<"t">>, <<>>) end,
                       fun() -> crypto:crypto_one_time_aead(C, <<>>, <<"t">>, <<>>) end]],
    [Ciphers, InitErrors, Errors].

cipher_order() ->
    Run = run_ciphers() ++ [aes_cbc, aes_ccm, aes_cfb128, aes_cfb8, aes_ctr, aes_ecb, aes_gcm],
    Order = [C || C <- crypto:supports(ciphers), lists:member(C, Run)],
    %% Run has no sm4_gcm and sm4_ccm, which the Linux CI host does not list.
    io:format("~p~n", [Order]).

%% The ciphers of the table cipher.c:30-173 that the OpenSSL 3 default
%% provider has and that the libcrypto of the Linux CI host also has. The
%% AEAD ciphers are the last seven. sm4_gcm and sm4_ccm are in
%% ciphers_host/0.
run_ciphers() ->
    [des_ede3_cbc, des_ede3_cfb, sm4_cbc, sm4_ecb, sm4_cfb, sm4_ofb, sm4_ctr,
     aes_128_cbc, aes_192_cbc, aes_256_cbc, aes_128_ofb, aes_192_ofb, aes_256_ofb,
     aes_128_cfb8, aes_192_cfb8, aes_256_cfb8, aes_128_cfb128, aes_192_cfb128,
     aes_256_cfb128, aes_128_ecb, aes_192_ecb, aes_256_ecb, aes_128_ctr, aes_192_ctr,
     aes_256_ctr, chacha20,
     chacha20_poly1305, aes_128_gcm, aes_192_gcm, aes_256_gcm,
     aes_128_ccm, aes_192_ccm, aes_256_ccm].

%% Write the results of cipher_info_nif/1, ng_crypto_one_time_nif/5 with
%% each padding, aead_cipher_nif/7, and do_exor/2 (cipher.c:266-371,
%% api_ng.c:91-664, 782-864, aead.c:150-377, math.c:25-54), with the error
%% tuple of each failed check. Random padding prints only its size.
ciphers() ->
    {Plain, Aead} = lists:split(26, run_ciphers()),
    Aliases = [aes_cbc, aes_ccm, aes_cfb128, aes_cfb8, aes_ctr, aes_ecb, aes_gcm],
    %% The type of des_ede3_cfb depends on the libcrypto of the host, so
    %% ciphers_host/0 prints the types.
    Infos = [bad_reason(fun() -> lists:sort(maps:to_list(maps:remove(type, crypto:cipher_info(C)))) end)
             || C <- run_ciphers() ++ Aliases ++ [nosuch, "aes_128_cbc"]],
    Data = <<"The quick brown fox jumps over th">>,
    OneTime = [one_time(C, Data) || C <- Plain],
    Aes = fun(Iv, D, Opts) -> bad_reason(fun() ->
              crypto:crypto_one_time(aes_128_cbc, binary:copy(<<1>>, 16), Iv, D, Opts) end) end,
    Iv = binary:copy(<<2>>, 16),
    Block = binary:copy(<<3>>, 16),
    Enc = crypto:crypto_one_time(aes_128_cbc, binary:copy(<<1>>, 16), Iv, Block, true),
    OneTimeErrors =
        [Aes(Iv, Block, Opts)
         || Opts <- [[], undefined, [{encrypt, undefined}, {padding, pkcs_padding}],
                     [{encrypt, true}, {encrypt, true}], [{padding, none}, {padding, zero}],
                     [{encrypt, perhaps}], [{padding, bad}], [{bad, 1}], [{"tag", 1}],
                     [{binary:copy(<<"long">>, 20), 1}], [x], [{encrypt}], [a | b], 1, perhaps]]
        ++ [Aes(Iv, D, [{encrypt, false}, {padding, pkcs_padding}]) || D <- [Enc, Block, <<1>>]]
        ++ [Aes(Iv, <<1>>, [{padding, none}]), Aes(<<1>>, Block, true), Aes(a, Block, true),
            Aes(undefined, Block, true), Aes(Iv, a, true), Aes(Iv, [Block, $a], true)]
        ++ [bad_reason(fun() -> crypto:crypto_one_time(C, K, Iv, Block, true) end)
            || {C, K} <- [{aes_128_cbc, <<1>>}, {aes_128_cbc, a}, {nosuch, <<1>>},
                          {"aes", <<1>>}, {aes_128_gcm, binary:copy(<<1>>, 16)},
                          {aes_cbc, <<1>>}]]
        ++ [bad_reason(fun() -> crypto:crypto_one_time(aes_128_ecb, binary:copy(<<1>>, 16),
                                                       a, Block, true) end),
            bad_reason(fun() -> crypto:crypto_one_time(aes_128_ecb, binary:copy(<<1>>, 16),
                                                       Block, true) end)],
    Aeads = [aead(C) || C <- Aead],
    Key = binary:copy(<<1>>, 16),
    Gcm = fun(K, I, T, A, TL, E) -> bad_reason(fun() ->
              crypto:crypto_one_time_aead(aes_128_gcm, K, I, T, A, TL, E) end) end,
    AeadErrors =
        [Gcm(Key, Iv, <<"t">>, <<>>, 16, perhaps), Gcm(a, Iv, <<"t">>, <<>>, 16, true),
         Gcm(Key, a, <<"t">>, <<>>, 16, true), Gcm(Key, Iv, a, <<>>, 16, true),
         Gcm(Key, Iv, <<"t">>, a, 16, true), Gcm(Key, Iv, <<"t">>, <<>>, -1, true),
         Gcm(Key, Iv, <<"t">>, <<>>, a, false), Gcm(<<1>>, Iv, <<"t">>, <<>>, 16, true),
         Gcm(Key, <<>>, <<"t">>, <<>>, 16, true), Gcm(Key, Iv, <<"t">>, <<>>, 17, true),
         Gcm(Key, Iv, <<"t">>, <<>>, 0, true), Gcm(Key, Iv, <<"t">>, <<>>, 16, undefined),
         Gcm(Key, Iv, <<"t">>, <<>>, <<>>, false), Gcm(Key, Iv, [<<"t">>], [<<"a">>], 4, true),
         bad_reason(fun() -> crypto:crypto_one_time_aead(aes_128_cbc, Key, Iv, <<"t">>,
                                                         <<>>, 16, true) end),
         bad_reason(fun() -> crypto:crypto_one_time_aead("gcm", Key, Iv, <<"t">>,
                                                         <<>>, 16, true) end),
         bad_reason(fun() -> crypto:crypto_one_time_aead(aes_128_ccm, Key, <<1:56>>,
                                                         <<"t">>, <<>>, 3, true) end),
         bad_reason(fun() -> crypto:crypto_one_time_aead(aes_128_ccm, Key, <<1:56>>,
                                                         <<"t">>, <<>>, 16, undefined) end),
         bad_reason(fun() -> crypto:crypto_one_time_aead(aes_128_ccm, Key, <<>>,
                                                         <<"t">>, <<>>, 16, true) end),
         bad_reason(fun() -> crypto:crypto_one_time_aead(chacha20_poly1305,
                                                         binary:copy(<<1>>, 32), <<1:104>>,
                                                         <<"t">>, <<>>, 16, true) end)],
    Exor = [bad_reason(fun() -> crypto:exor(A, B) end)
            || {A, B} <- [{<<1, 2, 3>>, <<3, 2, 1>>}, {<<>>, <<>>}, {[<<1>>, 2], <<3, 4>>},
                          {<<1>>, <<1, 2>>}, {a, <<1>>}, {<<1:7>>, <<1:7>>}]],
    io:format("~p~n", [[Infos, OneTime, OneTimeErrors, Aeads, AeadErrors, Exor]]).

one_time(Cipher, Data) ->
    #{key_length := KeyLength, iv_length := IvLength} = crypto:cipher_info(Cipher),
    Key = binary:copy(<<5>>, KeyLength),
    Iv = binary:copy(<<6>>, IvLength),
    Run = fun(D, Opts) -> bad_reason(fun() -> crypto:crypto_one_time(Cipher, Key, Iv, D, Opts) end) end,
    Pads = [Run(Data, [{padding, P}]) || P <- [undefined, none, zero, pkcs_padding]],
    Random = case Run(Data, [{padding, random}]) of
                 Bin when is_binary(Bin) -> byte_size(Bin);
                 Other -> Other
             end,
    Pkcs = Run(Data, [{padding, pkcs_padding}]),
    Back = case Pkcs of
               Bin2 when is_binary(Bin2) ->
                   [Run(Bin2, [{encrypt, false}, {padding, P}]) || P <- [pkcs_padding, none, undefined]];
               _ -> []
           end,
    {Cipher, Run(Data, true), Run(Data, false), Pads, Random, Back}.

aead(Cipher) ->
    #{key_length := KeyLength, iv_length := IvLength} = crypto:cipher_info(Cipher),
    Key = binary:copy(<<5>>, KeyLength),
    Iv = binary:copy(<<6>>, IvLength),
    Run = fun(T, A, TL, E) -> bad_reason(fun() ->
              crypto:crypto_one_time_aead(Cipher, Key, Iv, T, A, TL, E) end) end,
    Enc = Run(<<"plain text">>, <<"aad">>, 16, true),
    Short = Run(<<"plain text">>, <<"aad">>, 12, true),
    Default = bad_reason(fun() ->
                  crypto:crypto_one_time_aead(Cipher, Key, Iv, <<"text">>, <<"aad">>, true) end),
    Dec = case Enc of
              {Out, Tag} ->
                  [Run(Out, <<"aad">>, Tag, false), Run(Out, <<"bad">>, Tag, false),
                   Run(Out, <<"aad">>, binary:part(Tag, 0, 12), false)];
              _ -> []
          end,
    {Cipher, Enc, Short, Default, Dec}.

%% Write the results of the ciphers that depend on the libcrypto of the
%% host. The OpenSSL 3.5.8 of the pinned macOS ERTS cannot fetch rc2, rc4,
%% des, or blowfish (cipher.c:216-218), so these give notsup. The
%% libcrypto of the Linux CI host has no sm4_gcm and sm4_ccm. Its
%% des_ede3_cfb has the type 30, where OpenSSL 3.5.8 gives 61.
ciphers_host() ->
    Legacy = [rc2_cbc, rc4, des_cbc, des_cfb, des_ecb, blowfish_cbc, blowfish_cfb64,
              blowfish_ofb64, blowfish_ecb],
    Infos = [bad_reason(fun() -> crypto:cipher_info(C) end) || C <- Legacy],
    Sm4 = [aead(C) || C <- [sm4_gcm, sm4_ccm]],
    Types = [maps:get(type, crypto:cipher_info(C)) || C <- run_ciphers()],
    OneTime = [bad_reason(fun() -> crypto:crypto_one_time(C, binary:copy(<<1>>, 16),
                                                          binary:copy(<<2>>, 8),
                                                          <<"text">>, true) end)
               || C <- Legacy],
    io:format("~p~n", [[Infos, OneTime, Sm4, Types]]).

%% Call crypto:engine_list/0, whose NIF engine_get_first_nif/0 PBeam does
%% not link.
unsupported() ->
    io:format("~p~n", [crypto:engine_list()]).

bad(Fun) ->
    try Fun() catch Class:Reason:Stack -> {Class, Reason, hd(Stack)} end.

bad_reason(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.
