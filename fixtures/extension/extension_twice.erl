-module(extension_twice).
-export([load/0, twice/0]).

%% The extension library of the test lists `twice/0` two times.
-nifs([twice/0]).

load() -> erlang:load_nif("extension_twice", 0).

twice() -> erlang:nif_error(undef).
