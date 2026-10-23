-module(module_info_keys).
-export([zeta/0, alpha/1, main/0, stub/1]).

%% module_info/1 for the keys functions, nifs, and md5
%% (erl_bif_info.c:5732-5805, 5911-5944). The module has exports that are
%% not in name order, a local function, a fun, and a NIF stub that no
%% erlang:load_nif/2 replaces.
zeta() -> local(1).
alpha(X) -> fun() -> X end.
stub(_) -> erlang:nif_error(not_loaded).
local(X) -> X.
main() -> [module_info_keys:module_info(K) || K <- [functions, nifs, md5]].
