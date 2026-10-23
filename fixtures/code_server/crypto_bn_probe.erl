-module(crypto_bn_probe).
-export([main/0, kem_errors/0, kem/0, encapsulate/0, decapsulate/0]).

%% Write the results of crypto:mod_pow/3 and of the SRP functions of
%% crypto:generate_key/3 and crypto:compute_key/4, which call
%% mod_exp_nif/4, srp_value_B_nif/5, srp_user_secret_nif/7, and
%% srp_host_secret_nif/5 (crypto.erl:3820-3846, 3923-3927, 4566-4586,
%% bn.c:101-165, srp.c:26-300). The inputs are fixed, so the probe writes
%% the bytes.
main() ->
    T = fun(F) -> try F() catch C:R -> {C, R} end end,
    ModPow = [T(fun() -> crypto:mod_pow(B, E, M) end)
              || {B, E, M} <- [{2, 10, 17}, {5, 0, 23}, {5, 0, 1}, {3, 4, 0},
                               {<<200>>, 1, <<255>>}, {16#123456789, 16#abcdef, 16#fffffffb},
                               {7, 3, 10}, {0, 5, 7}, {a, 1, 2}, {1, a, 2}, {1, 2, a},
                               {-1, 2, 3}]],
    Prime = <<16#e3>>,
    Gen = <<2>>,
    Ver = <<16#4b>>,
    Srp = [T(F) || F <- [
        fun() -> crypto:generate_key(srp, {user, [Gen, Prime, '6a']}, <<7>>) end,
        fun() -> crypto:generate_key(srp, {host, [Ver, Gen, Prime, '6a']}, <<9>>) end,
        fun() -> crypto:generate_key(srp, {host, [Ver, Gen, Prime, '6']}, <<9>>) end,
        fun() -> crypto:generate_key(srp, {host, [Ver, Gen, Prime, '3']}, <<9>>) end,
        fun() -> crypto:generate_key(srp, {host, [<<0>>, <<0>>, Prime, '3']}, <<9>>) end,
        fun() -> crypto:generate_key(srp, {host, [Ver, Gen, <<16#e4>>, '3']}, <<9>>) end,
        fun() -> crypto:generate_key(srp, {host, [Ver, Gen, <<0>>, '3']}, <<9>>) end,
        fun() -> crypto:generate_key(srp, {host, [Ver, Gen, Prime, '3']}, a) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, Prime, Gen, '6a']}) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, Prime, Gen, '6', <<3>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, Prime, Gen, '3', <<1, 0, 0, 0, 0, 0, 0, 0, 1>>]}) end,
        fun() -> crypto:compute_key(srp, Prime, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, Prime, Gen, '3', <<3>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, <<16#e4>>, Gen, '3', <<3>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, <<0>>, Gen, '3', <<3>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [<<5>>, Prime, Gen, '3', a]}) end,
        fun() -> crypto:compute_key(srp, <<16#55>>, {<<16#21>>, <<7>>},
                                    {user, [a, Prime, Gen, '3', <<3>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, Prime, '6a']}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, Prime, '3', <<5>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, Prime, '3', <<1, 0, 0, 0, 0, 0, 0, 0, 5>>]}) end,
        fun() -> crypto:compute_key(srp, <<0>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, Prime, '3', <<5>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, <<16#e4>>, '3', <<5>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, <<0>>, '3', <<5>>]}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, <<9>>},
                                    {host, [Ver, Prime, '3', a]}) end,
        fun() -> crypto:compute_key(srp, <<16#33>>, {<<16#44>>, a},
                                    {host, [Ver, Prime, '3', <<5>>]}) end]],
    io:format("~p~n", [{ModPow, Srp}]).

%% The errors of crypto:encapsulate_key/2 and crypto:decapsulate_key/3
%% (crypto.erl:3756-3778, evp.c:30-129, pkey.c:587-665).
kem_errors() ->
    T = fun(F) -> try F() catch C:R -> {C, R} end end,
    Long = list_to_atom(lists:duplicate(40, $a)),
    io:format("~p~n", [[T(F) || F <- [
        fun() -> crypto:encapsulate_key(mlkem512, <<1, 2>>) end,
        fun() -> crypto:encapsulate_key('ML-KEM-768', <<1, 2>>) end,
        fun() -> crypto:encapsulate_key(mlkem1024, a) end,
        fun() -> crypto:encapsulate_key("mlkem512", <<1>>) end,
        fun() -> crypto:encapsulate_key(Long, <<1>>) end,
        fun() -> crypto:encapsulate_key(foo, <<1>>) end,
        fun() -> crypto:encapsulate_key(rsa, a) end,
        fun() -> crypto:decapsulate_key(mldsa44, a, <<>>) end,
        fun() -> crypto:decapsulate_key(mlkem768, <<1>>, a) end,
        fun() -> crypto:decapsulate_key(mlkem768, <<1>>, <<>>) end,
        fun() -> crypto:decapsulate_key(foo, a, <<>>) end]]]).

%% A shared secret of ML-KEM: the sizes, and the decapsulation round trip.
kem() ->
    io:format("~p~n", [[begin
        {Pub, Priv} = crypto:generate_key(Type, []),
        {Secret, Encaps} = crypto:encapsulate_key(Type, Pub),
        {Type, byte_size(Secret), byte_size(Encaps),
         crypto:decapsulate_key(Type, Priv, Encaps) =:= Secret}
    end || Type <- [mlkem512, mlkem768, mlkem1024]]]).

%% An encapsulation with a valid key (evp.c:45-63).
encapsulate() ->
    {Pub, _} = crypto:generate_key(mlkem512, []),
    crypto:encapsulate_key(mlkem512, Pub).

%% A decapsulation with a valid key (evp.c:99-116).
decapsulate() ->
    {_, Priv} = crypto:generate_key(mlkem512, []),
    crypto:decapsulate_key(mlkem512, Priv, <<1>>).
