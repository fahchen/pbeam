-module(crypto_probe).
-export([main/0, nifs/0, info/0, hash_init/0, hash_large/0]).

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
