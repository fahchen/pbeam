-module(loaded_order_probe).
-export([main/0]).

%% erlang:loaded/0 gives each module with current or old code, the last
%% module table index first (beam_bif_load.c:762-791). finish_loading/1
%% puts the module entry at the first load (beam_bif_load.c:425), and the
%% entry keeps its index after erlang:delete_module/1 and a later load.
%% The probe loads loaded_order_b before loaded_order_a, deletes
%% loaded_order_b, and loads it again. It gives only its own two modules
%% of each list, because the other loaded modules differ between ERTS and
%% PBeam.
main() ->
    {ok, A} = prim_file:read_file("fixtures/boot/loaded_order_a.beam"),
    {ok, B} = prim_file:read_file("fixtures/boot/loaded_order_b.beam"),
    {module, loaded_order_b} = erlang:load_module(loaded_order_b, B),
    {module, loaded_order_a} = erlang:load_module(loaded_order_a, A),
    Loaded = own(),
    true = erlang:delete_module(loaded_order_b),
    Deleted = own(),
    {module, loaded_order_b} = erlang:load_module(loaded_order_b, B),
    {Loaded, Deleted, own()}.

own() ->
    [Module || Module <- erlang:loaded(),
               Module =:= loaded_order_a orelse Module =:= loaded_order_b].
