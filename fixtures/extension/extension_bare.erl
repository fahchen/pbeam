-module(extension_bare).
-export([calls/0, add/2]).

%% An empty `-nifs` list: the compiler writes no `nif_start`, so ERTS can
%% replace each function of the library, and `add/2` keeps a BEAM body.
-nifs([]).

%% Call `add/2` before the load, load, and call it again.
calls() ->
    Before = add(1, 2),
    Load = erlang:load_nif("extension_bare", 0),
    {Before, Load, add(1, 2)}.

add(_A, _B) -> fallback.
