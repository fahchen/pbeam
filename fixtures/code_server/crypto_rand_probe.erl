-module(crypto_rand_probe).
-export([main/0]).
-compile(nowarn_deprecated_function).

%% Write the shape of the results of crypto:rand_uniform/2,
%% crypto:rand_seed/1, and the crypto plugins of rand, which call
%% rand_uniform_nif/2, rand_seed_nif/1, and strong_rand_bytes_nif/1
%% (rand.c:26-155, crypto.erl:2256-2300). The output is random, so the
%% probe writes the types, the sizes, and the ranges, and each error.
main() ->
    T = fun(F) -> try F() catch C:R -> {C, R} end end,
    Uniform = [{From, To, T(fun() ->
                                  V = crypto:rand_uniform(From, To),
                                  is_integer(V) andalso V >= From andalso V < To
                              end)}
               || {From, To} <- [{0, 1}, {0, 10}, {5, 6}, {-5, 5}, {-10, -9},
                                 {1 bsl 64, (1 bsl 64) + 3}, {0, 1 bsl 200},
                                 {255, 256}, {10, 10}, {10, 5}]],
    One = crypto:rand_uniform(7, 8),
    Mpint = fun(N) -> B = binary:encode_unsigned(N), <<(byte_size(B)):32, B/binary>> end,
    Bin = [T(fun() ->
                     <<Len:32, Rest/binary>> = R = crypto:rand_uniform(F, To),
                     {byte_size(Rest) =:= Len, Len, is_binary(R)}
             end)
           || {F, To} <- [{Mpint(128), Mpint(129)}, {Mpint(0), Mpint(1)},
                          {<<0:32>>, Mpint(2)}]],
    BinErrors = [T(fun() -> crypto:rand_uniform(F, To) end)
                 || {F, To} <- [{<<0:32>>, <<0:32>>}, {Mpint(5), Mpint(3)},
                                {<<1:32>>, Mpint(3)}, {<<1, 2, 3>>, Mpint(3)},
                                {<<-1:32, 1>>, Mpint(3)}, {Mpint(1), <<2:32, 1>>}]],
    Seed = [T(fun() -> crypto:rand_seed(S) end) || S <- [<<>>, <<"seed">>, a]],
    Plugins = [T(fun() ->
                         S = Seed0(),
                         {U, S1} = rand:uniform_s(10, S),
                         {F, S2} = rand:uniform_real_s(S1),
                         {B, _} = rand:bytes_s(5, S2),
                         {U >= 1 andalso U =< 10, is_float(F) andalso F < 1.0, byte_size(B)}
                 end)
               || Seed0 <- [fun crypto:rand_seed_s/0,
                            fun() -> crypto:rand_seed_alg_s(crypto) end,
                            fun() -> crypto:rand_seed_alg_s(crypto_cache) end,
                            fun() -> crypto:rand_seed_alg_s(crypto_cache, 64) end]],
    Process = [T(fun() -> crypto:rand_seed(), rand:uniform(3) =< 3 end),
               T(fun() -> crypto:rand_seed_alg(crypto_cache), byte_size(rand:bytes(9)) end)],
    io:format("~p~n", [[Uniform, One, Bin, BinErrors, Seed, Plugins, Process]]).
