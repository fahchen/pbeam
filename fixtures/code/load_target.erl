-module(load_target).
-export([value/0]).

%% The module that load_probe loads at run time. The atom load_target_value
%% is in no other fixture, so the load puts it into the atom table of the run.
value() -> load_target_value.
