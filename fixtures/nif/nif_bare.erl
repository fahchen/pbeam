-module(nif_bare).
-export([calls/1, add/2]).

%% An empty `-nifs` list: the compiler writes no `nif_start`, so ERTS can
%% replace each function of the library. The NIF_ADD_NAME=nif_bare variant
%% of nif_add.c implements add/2.
-nifs([]).

calls(Path) -> {erlang:load_nif(Path, 0), ?MODULE:add(1, 2)}.

add(_A, _B) -> fallback.
