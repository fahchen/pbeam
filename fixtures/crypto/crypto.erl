-module(crypto).
-export([load_info/0, load_ok/0, load_fips/0]).
-export([info_nif/0, info_lib/0, info_fips/0, enable_fips_mode_nif/1, hash_algorithms/0, pubkey_algorithms/0, cipher_algorithms/0, mac_algorithms/0, curve_algorithms/0, rsa_opts_algorithms/0, hash_info/1, hash_nif/2, hash_init_nif/1, hash_update_nif/2, hash_final_nif/1, hash_final_xof_nif/2, mac_nif/4, mac_init_nif/3, mac_update_nif/2, mac_final_nif/1, cipher_info_nif/1, ng_crypto_init_nif/4, ng_crypto_update_nif/2, ng_crypto_final_nif/1, ng_crypto_get_data_nif/1, ng_crypto_one_time_nif/5, strong_rand_bytes_nif/1, strong_rand_range_nif/1, rand_uniform_nif/2, mod_exp_nif/4, do_exor/2, hash_equals_nif/2, pbkdf2_hmac_nif/5, pkey_sign_nif/5, pkey_sign_heavy_nif/5, pkey_verify_nif/6, pkey_crypt_nif/6, encapsulate_key_nif/2, decapsulate_key_nif/3, kem_algorithms_nif/0, rsa_generate_key_nif/2, dh_generate_key_nif/4, dh_compute_key_nif/3, evp_compute_key_nif/3, evp_generate_key_nif/2, privkey_to_pubkey_nif/2, srp_value_B_nif/5, srp_user_secret_nif/7, srp_host_secret_nif/5, ec_generate_key_nif/2, ecdh_compute_key_nif/3, rand_seed_nif/1, aead_cipher_nif/7, aead_cipher_nif/4, aead_cipher_init_nif/4, engine_by_id_nif/1, engine_init_nif/1, engine_free_nif/1, engine_load_dynamic_nif/0, engine_ctrl_cmd_strings_nif/3, engine_register_nif/2, engine_unregister_nif/2, engine_add_nif/1, engine_remove_nif/1, engine_get_first_nif/0, engine_get_next_nif/1, engine_get_id_nif/1, engine_get_name_nif/1, engine_get_all_methods_nif/0, ensure_engine_loaded_nif/2]).
-nifs([info_nif/0, info_lib/0, info_fips/0, enable_fips_mode_nif/1, hash_algorithms/0, pubkey_algorithms/0, cipher_algorithms/0, mac_algorithms/0, curve_algorithms/0, rsa_opts_algorithms/0, hash_info/1, hash_nif/2, hash_init_nif/1, hash_update_nif/2, hash_final_nif/1, hash_final_xof_nif/2, mac_nif/4, mac_init_nif/3, mac_update_nif/2, mac_final_nif/1, cipher_info_nif/1, ng_crypto_init_nif/4, ng_crypto_update_nif/2, ng_crypto_final_nif/1, ng_crypto_get_data_nif/1, ng_crypto_one_time_nif/5, strong_rand_bytes_nif/1, strong_rand_range_nif/1, rand_uniform_nif/2, mod_exp_nif/4, do_exor/2, hash_equals_nif/2, pbkdf2_hmac_nif/5, pkey_sign_nif/5, pkey_sign_heavy_nif/5, pkey_verify_nif/6, pkey_crypt_nif/6, encapsulate_key_nif/2, decapsulate_key_nif/3, kem_algorithms_nif/0, rsa_generate_key_nif/2, dh_generate_key_nif/4, dh_compute_key_nif/3, evp_compute_key_nif/3, evp_generate_key_nif/2, privkey_to_pubkey_nif/2, srp_value_B_nif/5, srp_user_secret_nif/7, srp_host_secret_nif/5, ec_generate_key_nif/2, ecdh_compute_key_nif/3, rand_seed_nif/1, aead_cipher_nif/7, aead_cipher_nif/4, aead_cipher_init_nif/4, engine_by_id_nif/1, engine_init_nif/1, engine_free_nif/1, engine_load_dynamic_nif/0, engine_ctrl_cmd_strings_nif/3, engine_register_nif/2, engine_unregister_nif/2, engine_add_nif/1, engine_remove_nif/1, engine_get_first_nif/0, engine_get_next_nif/1, engine_get_id_nif/1, engine_get_name_nif/1, engine_get_all_methods_nif/0, ensure_engine_loaded_nif/2]).

%% A stand-in for the crypto module with each function of the nif_funcs
%% table of crypto.c. load_info/0 writes the results of erlang:load_nif/2
%% with the crypto NIF library for each load info. The load callback
%% checks the load info {302, LibBin, FipsMode}, and each failed check
%% gives the line of the check in crypto.c (crypto.c:204-218, 286-288;
%% erl_nif.c:4980-4986). An error does not load the library, so the next
%% call checks the next load info.
load_info() ->
    load_info(lists:seq(1, 6)).

%% Write the result of the load info that loads the library. ERTS stops
%% with a segmentation fault when this load follows the FipsMode check
%% failure of load_info/0, so it runs alone.
load_ok() ->
    load_info([7]).

%% Write the result of the load info with FipsMode true. The pinned ERTS
%% has no FIPS support and gives the veto 287; PBeam stops with a runtime
%% error.
load_fips() ->
    load_info([8]).

load_info(Indexes) ->
    Lib = filename:join([code:root_dir(), "lib", "crypto-" ++ vsn(), "priv", "lib", "crypto"]),
    Bin = list_to_binary(Lib),
    Infos = [none, {302, Bin}, {list_to_integer("18446744073709551616"), Bin, false}, {301, Bin, false},
             {302, Lib, false}, {302, Bin, yes}, {302, Bin, false},
             {302, Bin, true}],
    io:format("~p~n", [[erlang:load_nif(Lib, lists:nth(Index, Infos)) || Index <- Indexes]]).

vsn() ->
    [Dir] = filelib:wildcard("crypto-*", filename:join(code:root_dir(), "lib")),
    lists:nthtail(length("crypto-"), Dir).

info_nif() -> erlang:nif_error(undef).
info_lib() -> erlang:nif_error(undef).
info_fips() -> erlang:nif_error(undef).
enable_fips_mode_nif(_) -> erlang:nif_error(undef).
hash_algorithms() -> erlang:nif_error(undef).
pubkey_algorithms() -> erlang:nif_error(undef).
cipher_algorithms() -> erlang:nif_error(undef).
mac_algorithms() -> erlang:nif_error(undef).
curve_algorithms() -> erlang:nif_error(undef).
rsa_opts_algorithms() -> erlang:nif_error(undef).
hash_info(_) -> erlang:nif_error(undef).
hash_nif(_, _) -> erlang:nif_error(undef).
hash_init_nif(_) -> erlang:nif_error(undef).
hash_update_nif(_, _) -> erlang:nif_error(undef).
hash_final_nif(_) -> erlang:nif_error(undef).
hash_final_xof_nif(_, _) -> erlang:nif_error(undef).
mac_nif(_, _, _, _) -> erlang:nif_error(undef).
mac_init_nif(_, _, _) -> erlang:nif_error(undef).
mac_update_nif(_, _) -> erlang:nif_error(undef).
mac_final_nif(_) -> erlang:nif_error(undef).
cipher_info_nif(_) -> erlang:nif_error(undef).
ng_crypto_init_nif(_, _, _, _) -> erlang:nif_error(undef).
ng_crypto_update_nif(_, _) -> erlang:nif_error(undef).
ng_crypto_final_nif(_) -> erlang:nif_error(undef).
ng_crypto_get_data_nif(_) -> erlang:nif_error(undef).
ng_crypto_one_time_nif(_, _, _, _, _) -> erlang:nif_error(undef).
strong_rand_bytes_nif(_) -> erlang:nif_error(undef).
strong_rand_range_nif(_) -> erlang:nif_error(undef).
rand_uniform_nif(_, _) -> erlang:nif_error(undef).
mod_exp_nif(_, _, _, _) -> erlang:nif_error(undef).
do_exor(_, _) -> erlang:nif_error(undef).
hash_equals_nif(_, _) -> erlang:nif_error(undef).
pbkdf2_hmac_nif(_, _, _, _, _) -> erlang:nif_error(undef).
pkey_sign_nif(_, _, _, _, _) -> erlang:nif_error(undef).
pkey_sign_heavy_nif(_, _, _, _, _) -> erlang:nif_error(undef).
pkey_verify_nif(_, _, _, _, _, _) -> erlang:nif_error(undef).
pkey_crypt_nif(_, _, _, _, _, _) -> erlang:nif_error(undef).
encapsulate_key_nif(_, _) -> erlang:nif_error(undef).
decapsulate_key_nif(_, _, _) -> erlang:nif_error(undef).
kem_algorithms_nif() -> erlang:nif_error(undef).
rsa_generate_key_nif(_, _) -> erlang:nif_error(undef).
dh_generate_key_nif(_, _, _, _) -> erlang:nif_error(undef).
dh_compute_key_nif(_, _, _) -> erlang:nif_error(undef).
evp_compute_key_nif(_, _, _) -> erlang:nif_error(undef).
evp_generate_key_nif(_, _) -> erlang:nif_error(undef).
privkey_to_pubkey_nif(_, _) -> erlang:nif_error(undef).
srp_value_B_nif(_, _, _, _, _) -> erlang:nif_error(undef).
srp_user_secret_nif(_, _, _, _, _, _, _) -> erlang:nif_error(undef).
srp_host_secret_nif(_, _, _, _, _) -> erlang:nif_error(undef).
ec_generate_key_nif(_, _) -> erlang:nif_error(undef).
ecdh_compute_key_nif(_, _, _) -> erlang:nif_error(undef).
rand_seed_nif(_) -> erlang:nif_error(undef).
aead_cipher_nif(_, _, _, _, _, _, _) -> erlang:nif_error(undef).
aead_cipher_nif(_, _, _, _) -> erlang:nif_error(undef).
aead_cipher_init_nif(_, _, _, _) -> erlang:nif_error(undef).
engine_by_id_nif(_) -> erlang:nif_error(undef).
engine_init_nif(_) -> erlang:nif_error(undef).
engine_free_nif(_) -> erlang:nif_error(undef).
engine_load_dynamic_nif() -> erlang:nif_error(undef).
engine_ctrl_cmd_strings_nif(_, _, _) -> erlang:nif_error(undef).
engine_register_nif(_, _) -> erlang:nif_error(undef).
engine_unregister_nif(_, _) -> erlang:nif_error(undef).
engine_add_nif(_) -> erlang:nif_error(undef).
engine_remove_nif(_) -> erlang:nif_error(undef).
engine_get_first_nif() -> erlang:nif_error(undef).
engine_get_next_nif(_) -> erlang:nif_error(undef).
engine_get_id_nif(_) -> erlang:nif_error(undef).
engine_get_name_nif(_) -> erlang:nif_error(undef).
engine_get_all_methods_nif() -> erlang:nif_error(undef).
ensure_engine_loaded_nif(_, _) -> erlang:nif_error(undef).
