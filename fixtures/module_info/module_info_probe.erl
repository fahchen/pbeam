-module(module_info_probe).
-export([main/0, md5/0]).

%% erlang:pre_loaded/0 (erl_init.c:459-484), and module_info/1 and
%% erlang:get_module_info/2 for each argument shape
%% (erl_bif_info.c:5918-5945, 5986-6030).
main() ->
    Info = [try erlang:get_module_info(Module, Key) catch Class:Reason -> {Class, Reason} end
            || {Module, Key} <- [{module_info_probe, module}, {module_info_missing, module},
                                 {module_info_probe, other}, {"module_info_probe", module},
                                 {module_info_probe, "module"}]],
    [erlang:pre_loaded(), module_info_probe:module_info(module) | Info].

%% A key that PBeam does not implement yet.
md5() -> erlang:get_module_info(module_info_probe, md5).
