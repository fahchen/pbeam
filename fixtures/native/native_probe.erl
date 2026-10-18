-module(native_probe).
-export([run/0, invalid/0, local/0, load/0, double/1, add/2]).

-nifs([add/2, hidden/1]).

%% `double/1` is an ordinary BEAM function, so an unregistered MFA
%% resolves to it.
double(X) -> X + X.

%% `native_probe:add/2` is registered as a Rust native function by the
%% test, so the call below reaches Rust instead of a BEAM body.
run() ->
    {native_add, erlang:apply(native_probe, add, [1, 2]),
     erlang:apply(native_probe, double, [3])}.

invalid() ->
    try erlang:apply(native_probe, add, [1, a]) catch Class:Reason -> {Class, Reason} end.

%% A local call enters the frame and reaches the `nif_start` instruction,
%% so `add/2` and a private NIF stub run the registry too.
local() ->
    {add(1, 2), hidden(7)}.

%% Suppress the "nifs defined, but no call to erlang:load_nif/2" warning.
load() -> erlang:load_nif("native_probe", 0).

%% Stub body for the natively registered function. ERTS replaces it with the
%% native binding when the library loads.
add(_A, _B) -> erlang:nif_error(undef).

%% A private NIF stub. It has no export, so an external call cannot reach
%% it, but a local call enters its frame and runs the registry.
hidden(_X) -> erlang:nif_error(undef).
