%% A module without -nifs that calls erlang:load_nif/2 gets nif_start on each function, so the NIF manifest gate reports isprint/1, which is in the prim_tty library and has no binding.
-module(prim_tty).
-export([load/0, isatty/1, isprint/1]).

load() -> erlang:load_nif("prim_tty", 0).

isatty(_) -> erlang:nif_error(undef).
isprint(_) -> erlang:nif_error(undef).
