-module(code_probe).
-export([loaded/0, module_loaded/0, not_loaded/0]).

%% Call code:ensure_loaded/1 for loaded modules: this module, the code
%% module, and the erlang module. code:ensure_loaded/1 returns {module, Mod}
%% when erlang:module_loaded/1 returns true (code.erl:581-583).
loaded() ->
    [code:ensure_loaded(code_probe), code:ensure_loaded(code), code:ensure_loaded(erlang)].

%% Run erlang:module_loaded/1 on a loaded module, on an atom that names no
%% module, and on each argument that is not an atom.
module_loaded() ->
    [try erlang:module_loaded(M) catch Class:Reason -> {Class, Reason} end
     || M <- [code_probe, erlang, code_probe_missing, "code_probe", <<"code_probe">>, 1, {code_probe}]].

%% Call code:ensure_loaded/1 for a module that is not loaded.
not_loaded() -> code:ensure_loaded(code_probe_missing).
