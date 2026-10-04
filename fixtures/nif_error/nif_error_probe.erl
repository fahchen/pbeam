-module(nif_error_probe).
-export([main/0, stub/0, nif/0, load/0]).

-nifs([nif_stub/0]).

main() -> stub().

stub() -> erlang:nif_error(undef).

%% A function in `-nifs` starts with the `nif_start` instruction.
nif_stub() -> erlang:nif_error(undef).

nif() ->
    try nif_stub() catch Class:Reason -> {Class, Reason} end.

%% No caller. Without a call to `erlang:load_nif/2`, `erl_lint` gives the
%% warning "nifs defined, but no call to erlang:load_nif/2", and `-Werror`
%% stops the compile.
load() -> erlang:load_nif("nif_error_probe", 0).
