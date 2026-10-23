-module(crypto_engine_probe).
-export([main/0, by_id/0, load/0, ensure/0]).

%% Write the result of each public engine function of crypto
%% (crypto.erl:3996-4415), which call the engine NIFs (engine.c:353-1045).
%% The reference is not an engine, so each NIF that takes an engine
%% gives badarg.
main() ->
    T = fun(F) -> try F() catch C:R -> {C, R} end end,
    R = make_ref(),
    io:format("~p~n", [[T(F) || F <- [
        fun() -> crypto:engine_get_all_methods() end,
        fun() -> crypto:engine_list() end,
        fun() -> crypto:engine_unload(R) end,
        fun() -> crypto:engine_add(R) end,
        fun() -> crypto:engine_remove(R) end,
        fun() -> crypto:engine_register(R, [engine_method_rsa]) end,
        fun() -> crypto:engine_unregister(R, [engine_method_rsa]) end,
        fun() -> crypto:engine_get_id(R) end,
        fun() -> crypto:engine_get_name(R) end,
        fun() -> crypto:engine_ctrl_cmd_string(R, <<"a">>, <<"b">>) end,
        fun() -> crypto:engine_by_id(a) end,
        fun() -> crypto:engine_by_id(<<1:1>>) end,
        fun() -> crypto:ensure_engine_loaded(a, <<"y">>) end,
        fun() -> crypto:ensure_engine_loaded(<<"x">>, a) end]]]).

%% Look up an engine id.
by_id() ->
    io:format("~p~n", [crypto:engine_by_id(<<"x">>)]).

%% Load an engine with the engine dynamic.
load() ->
    io:format("~p~n", [crypto:engine_load(<<"x">>, [], [])]).

%% Load an engine from a library path.
ensure() ->
    io:format("~p~n", [crypto:ensure_engine_loaded(<<"x">>, <<"y">>)]).
