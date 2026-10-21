%% The NIF manifest gate reports the library of this module, which has no PBeam bindings.
-module(nif_gate_library).
-export([load/0, f/0]).
-nifs([f/0]).

load() -> erlang:load_nif("priv/nif_gate_library", 0).

f() -> erlang:nif_error(undef).
