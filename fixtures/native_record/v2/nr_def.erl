-module(nr_def).
-export([pub/1]).

%% The second version: record pub is exported and has the fields b and c.
-export_record([pub]).
-record #pub{b = 2, c = 3}.

pub(C) ->
    #pub{c = C}.
