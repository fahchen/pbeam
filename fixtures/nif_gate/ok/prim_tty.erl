%% The NIF manifest gate passes this module: each declared function of the prim_tty library has a binding, and extra/0 is not in the library.
-module(prim_tty).
-export([load/0, isatty/1, write_nif/2, extra/0]).
-nifs([isatty/1, write_nif/2, extra/0]).

load() -> erlang:load_nif(atom_to_list(?MODULE), 0).

isatty(_) -> erlang:nif_error(undef).
write_nif(_, _) -> erlang:nif_error(undef).
extra() -> erlang:nif_error(undef).
