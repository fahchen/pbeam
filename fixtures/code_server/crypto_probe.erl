-module(crypto_probe).
-export([main/0, nifs/0, info/0, supports/0, fips/0, oneshot/0, xof_sizes/0, hash_init/0, hash_large/0]).

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
           ripemd160, md5, cmac, hmac, poly1305],
    All = crypto:supports(),
    Items = [{Type, [Item || Item <- List, lists:member(Item, Run)]} || {Type, List} <- All],
    Lists = [crypto:supports(Type) || {Type, _} <- All],
    io:format("~p~n", [[length(lists:append([List || {_, List} <- All])), Items,
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
    Digests = [md4, md5, ripemd160, sha, sha224, sha256, sha384, sha512, sha512_224,
               sha512_256, sha3_224, sha3_256, sha3_384, sha3_512, shake128, shake256,
               sm3, blake2b, blake2s],
    %% The size of a SHAKE digest depends on the libcrypto of the host, so
    %% the probe xof_sizes/0 prints it apart.
    Infos = [bad_reason(fun() -> maps:remove(size, crypto:hash_info(Type)) end)
             || Type <- Digests ++ [nosuch, 1]],
    Hashes = [bad_reason(fun() -> crypto:hash(Type, <<"abc">>) end) || Type <- Digests],
    Hmacs = [bad_reason(fun() -> crypto:mac(hmac, Type, <<"key">>, [<<"te">>, $x, "t"]) end)
             || Type <- Digests ++ [nosuch, 1]],
    Ciphers = [{rc2_cbc, 16}, {rc4, 16}, {des_cbc, 8}, {des_cfb, 8}, {des_ecb, 8},
               {des_ede3_cbc, 24}, {des_ede3_cbc, 16}, {des_ede3_cfb, 24},
               {blowfish_cbc, 16}, {blowfish_cfb64, 16}, {blowfish_ofb64, 16},
               {blowfish_ecb, 16}, {sm4_cbc, 16}, {sm4_ecb, 16}, {sm4_cfb, 16},
               {sm4_ofb, 16}, {sm4_ctr, 16}, {sm4_gcm, 16}, {sm4_ccm, 16},
               {aes_128_cbc, 16}, {aes_192_cbc, 24}, {aes_256_cbc, 32},
               {aes_128_ofb, 16}, {aes_128_cfb8, 16}, {aes_128_cfb128, 16},
               {aes_128_ecb, 16}, {aes_128_ctr, 16}, {chacha20, 32},
               {chacha20_poly1305, 32}, {aes_128_gcm, 16}, {aes_256_ccm, 32},
               {aes_cbc, 24}, {aes_ecb, 16}, {aes_128_cbc, 8}, {sm4_cbc, 8},
               {nosuch, 16}, {"aes_128_cbc", 16}],
    Cmacs = [{Cipher, bad_reason(fun() ->
                                         crypto:mac(cmac, Cipher, binary:copy(<<7>>, Size), <<"text">>)
                                 end)}
             || {Cipher, Size} <- Ciphers],
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

%% Write the sizes of hash_info/1 for the SHAKE digests. A fetched md gives
%% 0 for an XOF (evp_lib.c:1041); the legacy EVP_shake* gives bitlen/8.
xof_sizes() ->
    io:format("~p~n", [[maps:get(size, crypto:hash_info(Type)) || Type <- [shake128, shake256]]]).

%% Call crypto:hash_init/1, whose NIF hash_init_nif/1 PBeam does not link.
hash_init() ->
    io:format("~p~n", [crypto:hash_init(sha)]).

%% Call crypto:hash/2 with more than the 20000 bytes that hash_nif/2 takes.
%% crypto:hash/2 sends the input through hash_init_nif/1, which PBeam does
%% not link (crypto.erl:4535-4540).
hash_large() ->
    io:format("~p~n", [crypto:hash(sha, binary:copy(<<0>>, 20001))]).

bad(Fun) ->
    try Fun() catch Class:Reason:Stack -> {Class, Reason, hd(Stack)} end.

bad_reason(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.
