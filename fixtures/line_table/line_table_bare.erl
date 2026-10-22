-module(line_table_bare).
-export([call/1, call/2]).

%% The fixture has no line table, so ERTS gives each frame of this module
%% the location [].
call(F) ->
    [F()].

call(F, Kind) ->
    [F(Kind)].
