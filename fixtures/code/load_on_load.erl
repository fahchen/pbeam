-module(load_on_load).
-export([value/0]).
-on_load(init/0).

%% A module with an -on_load function, which the code server runs after
%% erlang:finish_loading/1 returns {on_load, [load_on_load]}.
init() -> ok.

value() -> load_on_load_value.
