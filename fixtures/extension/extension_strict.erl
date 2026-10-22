-module(extension_strict).
-export([load/0, unbound/0]).

%% The extension library of the test lists `unbound/0` two times and binds
%% no function, and a call of a function without a binding fails.
-nifs([unbound/0]).

load() -> erlang:load_nif("extension_strict", 0).

unbound() -> erlang:nif_error(undef).
