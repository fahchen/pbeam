-module(module_info_chunks).
-export([main/0]).
-vsn(1).
-probe_attribute([{key, <<"value">>}, -3]).

%% module_info/1 for the keys attributes, compile, native_addresses, and
%% native (erl_bif_info.c:5807-5909, 5918-5945). The Attr and CInf chunks
%% hold one term each (jit/asm_load.c:1080-1111).
main() ->
    [module_info_chunks:module_info(K)
     || K <- [attributes, compile, native_addresses, native]].
