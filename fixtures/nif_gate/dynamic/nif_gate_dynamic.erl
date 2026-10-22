%% The NIF manifest gate cannot resolve the library name of this module: the path of load/1 comes from the argument, and the literal path of load_list/0 is a list that is not a string.
-module(nif_gate_dynamic).
-export([load/1, load_list/0]).

load(Path) -> erlang:load_nif(Path, 0).

load_list() -> erlang:load_nif([priv], 0).
