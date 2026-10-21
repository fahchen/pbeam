-module(asn1rt_nif).
-export([load/2]).
-export([encode_per_complete/1, decode_ber_tlv_raw/1, encode_ber_tlv/1]).
-nifs([encode_per_complete/1, decode_ber_tlv_raw/1, encode_ber_tlv/1]).

%% A stand-in for the asn1rt_nif module with each function of the nif_funcs
%% table of its NIF library (asn1_erl_nif.c:1343-1347). load/2 returns the result of
%% erlang:load_nif/2 for each load info, in order. An error does not load
%% the library, so the next load info runs the checks again.
load(Lib, Infos) ->
    [erlang:load_nif(Lib, Info) || Info <- Infos].

encode_per_complete(_) -> erlang:nif_error(undef).
decode_ber_tlv_raw(_) -> erlang:nif_error(undef).
encode_ber_tlv(_) -> erlang:nif_error(undef).
