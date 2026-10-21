-module(nif_library_probe).
-export([main/0, pinned/0, frame_header/0]).

%% Load the stand-in of each module of an OTP NIF library that PBeam has
%% no bindings for, and write the results of erlang:load_nif/2 for each
%% load info. When the pinned module is loaded, code:delete/1 and
%% code:purge/1 remove it first, so the stand-in makes the first load of
%% the library and not an upgrade (erl_nif.c:4956-4979). The load callback of zstd, net, and
%% erl_tracer returns 0 (zstd_nif.c:1008-1070, prim_net_nif.c:5116-5146,
%% erl_tracer_nif.c:89-99). The load callback of prim_socket needs a map
%% with a local pid under the key registry (prim_socket_nif.c:18862-18867),
%% and the load callback of asn1rt_nif needs the integer 1
%% (asn1_erl_nif.c:1320-1329).
main() ->
    Asn1 = filename:join([code:priv_dir(asn1), "lib", "asn1rt_nif"]),
    Cases = [{zstd, "zstd", [zstd, zstd]},
             {prim_socket, "prim_socket",
              [0, #{}, #{registry => a}, #{registry => self()}, #{registry => self()}]},
             {prim_net, "net", [#{}, #{}]},
             {erl_tracer, "erl_tracer", [0, 0]},
             {asn1rt_nif, Asn1, [0, 2, a, 1 bsl 64, 1.0, 1, 1]}],
    [io:format("~p~n", [{Module, load(Module, Lib, Infos)}]) || {Module, Lib, Infos} <- Cases],
    ok.

load(Module, Lib, Infos) ->
    code:unstick_mod(Module),
    case erlang:module_loaded(Module) of
        true ->
            code:delete(Module),
            code:purge(Module);
        false ->
            ok
    end,
    Path = filename:join("fixtures/nif_libraries", atom_to_list(Module)),
    {module, Module} = code:load_abs(Path),
    Module:load(Lib, Infos).

%% Load the pinned zstd module. Its on_load function loads the zstd NIF
%% library and matches ok on the result (zstd.erl:251-252), so each
%% function of the library is defined in the module.
pinned() ->
    io:format("~p~n", [code:ensure_loaded(zstd)]).

%% Call zstd:get_frame_header/1, which calls get_frame_header_nif/1
%% (zstd.erl:433-435).
frame_header() ->
    io:format("~p~n", [zstd:get_frame_header(<<>>)]).
