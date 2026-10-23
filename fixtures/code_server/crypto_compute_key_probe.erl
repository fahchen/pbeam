-module(crypto_compute_key_probe).
-export([main/0, unsupported_field/0, unsupported_length/0, p1024/0]).

%% Write the results of crypto:generate_key/2,3 for dh and of
%% crypto:compute_key/4, which call dh_generate_key_nif/4,
%% dh_compute_key_nif/3, ecdh_compute_key_nif/3, and evp_compute_key_nif/3
%% (crypto.erl:3664-3674, 3813-3864, dh.c:44-271, ecdh.c:38-141,
%% evp.c:131-218). A new DH key is random, so the probe writes its sizes
%% and whether both sides compute the same secret. A shared secret from
%% given keys does not depend on a random value, so the probe writes its
%% bytes. An error writes its reason and its error_info.
main() ->
    G = <<2>>,
    Dh = [dh(P, G, L) || P <- [p1024(), p2048()], L <- [0]],
    DhGiven = [begin
                   {Pub, Priv} = crypto:generate_key(dh, [p1024(), G], K),
                   {byte_size(Pub) =< 128, byte_size(Priv) =< 128}
               end || K <- [<<7>>, 7, <<0>>]],
    DhErrors = [catch_error(fun() -> element(1, crypto:generate_key(dh, Params, K)) end)
                || {Params, K} <- [{[p1024(), G], a}, {[a, G], undefined},
                                   {[p1024(), a], undefined},
                                   {[p1024(), G, -1], undefined},
                                   {[p1024(), G, a], undefined},
                                   {[p1024(), G, 1 bsl 64], undefined},
                                   {[p1024(), G, 1 bsl 63], undefined},
                                   {[<<23>>, <<5>>], undefined},
                                   {[<<>>, G], undefined}, {[p1024(), <<>>], undefined},
                                   {[p1024(), <<1>>], undefined},
                                   {[p1024(), <<0>>], undefined}]],
    DhCompute = [catch_error(fun() -> crypto:compute_key(dh, Pub, Priv, Params) end)
                 || {Pub, Priv, Params} <-
                        [{<<5>>, <<3>>, [p1024(), G]}, {5, 3, [p1024(), G]},
                         {binary:copy(<<7>>, 100), binary:copy(<<9>>, 20),
                          [p1024(), G]},
                         {<<1>>, <<3>>, [p1024(), G]}, {<<0>>, <<3>>, [p1024(), G]},
                         {p1024(), <<3>>, [p1024(), G]},
                         {minus1(p1024()), <<3>>, [p1024(), G]},
                         {<<5>>, <<0>>, [p1024(), G]}, {<<5>>, p1024(), [p1024(), G]},
                         {<<5>>, <<3>>, [<<23>>, <<5>>]},
                         {a, <<3>>, [p1024(), G]}, {<<5>>, a, [p1024(), G]},
                         {<<5>>, <<3>>, [a, G]}, {<<5>>, <<3>>, [p1024(), a]}]],
    Ecdh = [ecdh(Curve) || Curve <- [secp256r1, secp384r1, secp521r1, brainpoolP256r1,
                                     secp256k1, prime192v1,
                                     crypto_ec_curves:curve(secp256r1)]],
    {Pub2, _} = crypto:generate_key(ecdh, secp256r1, 2),
    Def = crypto_ec_curves:curve(secp256r1),
    EcdhErrors = [catch_error(fun() -> crypto:compute_key(ecdh, Others, My, Curve) end)
                  || {Others, My, Curve} <-
                         [{a, <<1>>, secp256r1}, {Pub2, a, secp256r1},
                          {Pub2, <<1>>, foo}, {<<4, 0:512>>, a, secp256r1},
                          {<<4, 0:512>>, <<1>>, secp256r1}, {<<>>, <<1>>, secp256r1},
                          {Pub2, <<0>>, secp256r1}, {Pub2, 0, secp256r1},
                          {Pub2, binary:copy(<<255>>, 32), secp256r1},
                          {Pub2, binary:copy(<<255>>, 33), secp256r1},
                          {compress(Pub2), <<1>>, secp256r1},
                          {<<4, 0:512>>, <<1>>, Def},
                          {Pub2, <<1>>, setelement(3, Def, a)},
                          {Pub2, <<1>>, setelement(4, Def, a)},
                          {Pub2, <<1>>, setelement(5, Def, none)},
                          {Pub2, <<1>>, setelement(5, Def, a)},
                          {Pub2, <<1>>, setelement(2, Def, a)},
                          {Pub2, <<1>>, setelement(2, Def, {a, <<1>>, none})},
                          {Pub2, <<1>>, setelement(2, Def, {<<1>>, a, none})},
                          {Pub2, <<1>>, setelement(1, Def, a)},
                          {Pub2, <<1>>, setelement(1, Def, {prime_field, a})},
                          {Pub2, <<1>>, setelement(1, Def, {foo, <<1>>})},
                          {Pub2, <<1>>, {{prime_field, <<4>>}, {<<1>>, <<1>>, none},
                                         <<4, 1, 1>>, <<5>>, <<1>>}}]],
    Evp = [evp(Type, Curve) || {Type, Curve} <- [{eddh, x25519}, {eddh, x448},
                                                 {ecdh, x25519}, {ecdh, x448}]],
    EvpErrors = [catch_error(fun() -> crypto:compute_key(eddh, Others, My, Curve) end)
                 || {Others, My, Curve} <-
                        [{binary:copy(<<9>>, 32), a, x25519},
                         {binary:copy(<<9>>, 32), <<1, 2>>, x25519},
                         {a, binary:copy(<<1>>, 32), x25519},
                         {<<1>>, binary:copy(<<1>>, 32), x25519},
                         {<<0:256>>, binary:copy(<<1>>, 32), x25519},
                         {<<1:256/little>>, binary:copy(<<1>>, 32), x25519},
                         {<<0:448>>, binary:copy(<<1>>, 56), x448},
                         {9, 1, x25519}]],
    io:format("~p~n", [[Dh, DhGiven, DhErrors, DhCompute, Ecdh, EcdhErrors, Evp,
                        EvpErrors]]).

%% A private key length (dh.c:93-103).
unsupported_length() ->
    crypto:generate_key(dh, [p1024(), <<2>>, 100]).

%% A curve over a characteristic-two field (ec.c:141-215).
unsupported_field() ->
    crypto:compute_key(ecdh, <<4, 1, 2>>, <<1>>, crypto_ec_curves:curve(sect163k1)).

dh(P, G, L) ->
    Params = case L of 0 -> [P, G]; _ -> [P, G, L] end,
    Keys = [crypto:generate_key(dh, Params) || _ <- lists:seq(1, 8)],
    [{A, KA}, {B, KB} | _] = Keys,
    Bits = [bit_length(unsigned(Priv)) || {_, Priv} <- Keys],
    {byte_size(P), lists:max([byte_size(Pub) || {Pub, _} <- Keys]) =< byte_size(P),
     L =:= 0 orelse lists:max(Bits) =< L,
     crypto:compute_key(dh, B, KA, [P, G]) =:= crypto:compute_key(dh, A, KB, [P, G])}.

%% A private key on a curve without a name stops PBeam key generation
%% (ec.c:465-482), so the keys of such a curve come from its named curve.
ecdh(Curve) ->
    Named = case Curve of {_, _, _, _, _} -> secp256r1; _ -> Curve end,
    {PubA, PrivA} = crypto:generate_key(ecdh, Named, 3),
    {PubB, PrivB} = crypto:generate_key(ecdh, Named, binary:copy(<<5>>, 8)),
    S = crypto:compute_key(ecdh, PubB, PrivA, Curve),
    {S, S =:= crypto:compute_key(ecdh, PubA, PrivB, Curve),
     crypto:compute_key(ecdh, PubB, 3, Curve) =:= S}.

evp(Type, Curve) ->
    Size = case Curve of x25519 -> 32; x448 -> 56 end,
    {PubA, PrivA} = crypto:generate_key(eddh, Curve, binary:copy(<<1>>, Size)),
    {PubB, PrivB} = crypto:generate_key(eddh, Curve, binary:copy(<<7>>, Size)),
    S = crypto:compute_key(Type, PubB, PrivA, Curve),
    {S, S =:= crypto:compute_key(Type, PubA, PrivB, Curve)}.

compress(<<4, X:32/binary, _:31/binary, Last>>) ->
    <<(2 + (Last band 1)), X/binary>>.

minus1(P) ->
    Size = bit_size(P),
    <<N:Size>> = P,
    <<(N - 1):Size>>.

unsigned(Bin) ->
    Size = bit_size(Bin),
    <<N:Size>> = Bin,
    N.

bit_length(0) -> 0;
bit_length(N) -> 1 + bit_length(N bsr 1).

p1024() ->
    <<16#FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF:1024>>.

p2048() ->
    <<16#FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6955817183995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF:2048>>.

catch_error(F) ->
    try F() catch error:Reason:Stack ->
        [{_, _, _, Info} | _] = Stack,
        {Reason, proplists:get_value(error_info, Info)}
    end.
