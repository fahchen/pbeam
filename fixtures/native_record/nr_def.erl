-module(nr_def).
-export([pub/1, priv/0]).

%% An exported record with a field that has no default, and a private
%% record.
-export_record([pub]).
-record #pub{b = 1, a}.
-record #priv{z = 1}.

pub(A) ->
    #pub{a = A}.

priv() ->
    #priv{}.
