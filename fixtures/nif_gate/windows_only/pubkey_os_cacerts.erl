%% The NIF manifest gate passes this module on a host that is not Windows: as the pinned pubkey_os_cacerts.erl:265-275, on_load/0 calls erlang:load_nif/2 with a path that it builds at run time only on Windows, and the module has no PBeam library.
-module(pubkey_os_cacerts).
-export([os_cacerts/0]).
-on_load(on_load/0).

os_cacerts() -> erlang:nif_error(nif_not_loaded).

on_load() ->
    case os:type() of
        {win32, _} -> load_nif();
        _ -> ok
    end.

load_nif() ->
    Lib = filename:join([code:priv_dir(public_key), "lib", "public_key"]),
    erlang:load_nif(Lib, 0).
