-module(extension_strict).
-export([load/0, unbound/0]).

%% The extension library of the test binds no function, and a call of a
%% function without a binding fails after the load.
-nifs([unbound/0]).

load() ->
    ok = erlang:load_nif("extension_strict", 0),
    unbound().

unbound() -> erlang:nif_error(undef).
