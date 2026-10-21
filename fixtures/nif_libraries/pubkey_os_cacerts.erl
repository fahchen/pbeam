-module(pubkey_os_cacerts).
-export([load/2]).
-export([os_cacerts/0]).
-nifs([os_cacerts/0]).

%% A stand-in for the pubkey_os_cacerts module with each function of the nif_funcs
%% table of its NIF library (public_key.c:64-67). load/2 returns the result of
%% erlang:load_nif/2 for each load info, in order. An error does not load
%% the library, so the next load info runs the checks again.
load(Lib, Infos) ->
    [erlang:load_nif(Lib, Info) || Info <- Infos].

os_cacerts() -> erlang:nif_error(undef).
