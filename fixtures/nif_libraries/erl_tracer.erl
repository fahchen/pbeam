-module(erl_tracer).
-export([load/2]).
-export([enabled/3, trace/5]).
-nifs([enabled/3, trace/5]).

%% A stand-in for the erl_tracer module with each function of the nif_funcs
%% table of its NIF library (erl_tracer_nif.c:49-52). load/2 returns the result of
%% erlang:load_nif/2 for each load info, in order. An error does not load
%% the library, so the next load info runs the checks again.
load(Lib, Infos) ->
    [erlang:load_nif(Lib, Info) || Info <- Infos].

enabled(_, _, _) -> erlang:nif_error(undef).
trace(_, _, _, _, _) -> erlang:nif_error(undef).
