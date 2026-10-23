-module(asn1rt_nif).
-export([load/1, nifs/1, encode_per_complete/1, decode_ber_tlv/1, encode_ber_tlv/1]).
-nifs([encode_per_complete/1, decode_ber_tlv_raw/1, encode_ber_tlv/1]).

%% A stand-in for the asn1rt_nif module of OTP 29.1.1 with its exported
%% functions (asn1rt_nif.erl:28-30, 97-103). load/1 loads the NIF library
%% from the path Lib with the load info 1, as load_nif/0 does
%% (asn1rt_nif.erl:67), so the probe runs without the code server.
load(Lib) ->
    erlang:load_nif(Lib, 1).

%% Load the library and return the NIFs of the module
%% (erl_bif_info.c:5769-5805).
nifs(Lib) ->
    ok = load(Lib),
    erlang:get_module_info(asn1rt_nif, nifs).

decode_ber_tlv(Binary) ->
    case decode_ber_tlv_raw(Binary) of
        {error, Reason} ->
            exit({error, {asn1, Reason}});
        Other ->
            Other
    end.

encode_per_complete(_) -> erlang:nif_error(undef).
decode_ber_tlv_raw(_) -> erlang:nif_error(undef).
encode_ber_tlv(_) -> erlang:nif_error(undef).
