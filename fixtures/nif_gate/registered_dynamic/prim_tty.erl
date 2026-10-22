%% The NIF manifest gate passes this module: prim_tty has a PBeam library, which the module selects, so the path of load/1, which comes from the argument, need not be a literal string.
-module(prim_tty).
-export([load/1, isatty/1]).
-nifs([isatty/1]).

load(Path) -> erlang:load_nif(Path, 0).

isatty(_) -> erlang:nif_error(undef).
