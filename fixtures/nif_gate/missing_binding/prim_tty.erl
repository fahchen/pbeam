%% The NIF manifest gate reports wcwidth/1 and isprint/1, which are in the prim_tty library and have no binding.
-module(prim_tty).
-export([load/0, wcwidth/1, isatty/1, isprint/1]).
-nifs([wcwidth/1, isatty/1, isprint/1]).

load() -> erlang:load_nif("prim_tty", 0).

wcwidth(_) -> erlang:nif_error(undef).
isatty(_) -> erlang:nif_error(undef).
isprint(_) -> erlang:nif_error(undef).
