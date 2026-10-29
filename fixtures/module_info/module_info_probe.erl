-module(module_info_probe).
-export([main/0, exports/0, info/0]).

%% erlang:pre_loaded/0 (erl_init.c:459-484), and module_info/1 and
%% erlang:get_module_info/2 for each argument shape
%% (erl_bif_info.c:5918-5945, 5986-6030).
main() ->
    Info = [try erlang:get_module_info(Module, Key) catch Class:Reason -> {Class, Reason} end
            || {Module, Key} <- [{module_info_probe, module}, {module_info_missing, module},
                                 {module_info_probe, other}, {"module_info_probe", module},
                                 {module_info_probe, "module"}]],
    Info1 = [try erlang:get_module_info(M) catch Class:Reason -> {Class, Reason} end
             || M <- [module_info_missing, "module_info_probe"]],
    [erlang:pre_loaded(), module_info_probe:module_info(module), Info1 | Info].

%% exported_from_module prepends each export of the module in the order of
%% the export table (erl_bif_info.c:5822-5862), where the loader puts the
%% exports in ExpT order (jit/asm_load.c:1180-1186).
exports() -> erlang:get_module_info(module_info_probe, exports).

%% erlang:get_module_info/1, which Module:module_info/0 calls
%% (erl_bif_info.c:5946-5984, 6010-6019): the pairs of the keys module,
%% exports, attributes, compile, and md5.
info() -> erlang:get_module_info(module_info_probe).
