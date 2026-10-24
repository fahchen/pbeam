-module(nr_ext).
-export([results/0, show/0]).

-record #point{x = 0, y}.
-record #empty{}.

%% Return the external format of native records: term_to_binary/1,2,
%% external_size/1,2, and binary_to_term/1,2 with their errors.
results() ->
    P = #point{x = id(1), y = id([a, {b}])},
    E = #empty{},
    Pub = nr_def:pub(id(5)),
    Other = records:create(nr_ext, other, [{zb, 1}, {za, 2}], #{is_exported => true}),
    None = records:create(nr_none, point, [{y, 2}, {x, 1}], #{is_exported => false}),
    Records = [P, E, Pub, Other, None, {P, [Pub]}],
    Bins = [term_to_binary(R) || R <- Records],
    Back = [binary_to_term(B) || B <- Bins],
    [Bins,
     [erlang:external_size(R) || R <- Records],
     term_to_binary(P, [compressed]),
     term_to_binary(Pub, [deterministic, {minor_version, 1}]),
     erlang:external_size(Pub, [{minor_version, 0}]),
     [B =:= R || {B, R} <- lists:zip(Back, Records)],
     [B == R || {B, R} <- lists:zip(Back, Records)],
     [records:get_field_names(B) || B <- lists:sublist(Back, 5)],
     [records:is_exported(B) || B <- lists:sublist(Back, 5)],
     records:get(y, hd(Back)),
     binary_to_term(term_to_binary(P), [safe]),
     binary_to_term(<<(term_to_binary(P))/binary, 7>>, [used]),
     %% {nr_ext, point} with is_exported = true is not the loaded definition.
     catch_b2t(<<131, 67, 0:32, 1, 119, 6, "nr_ext", 119, 5, "point">>) =:= P,
     catch_b2t(<<131, 67, 2:32, 1, 119, 6, "nr_ext", 119, 5, "point",
                 119, 1, "y", 119, 1, "x", 97, 2, 97, 1>>),
     records:get_field_names(catch_b2t(<<131, 67, 2:32, 0, 119, 6, "nr_ext", 119, 5, "point",
                                          119, 1, "y", 119, 1, "x", 97, 2, 97, 1>>)),
     catch_b2t(<<131, 67, 2:32, 0, 119, 6, "nr_ext", 119, 5, "point",
                 119, 1, "y", 119, 1, "x", 97, 2, 97, 1>>) =:= #point{x = 1, y = 2},
     catch_b2t(<<131, 67, 0:32, 2, 119, 1, "m", 119, 1, "n">>),
     catch_b2t(<<131, 67, 2:32, 0, 119, 1, "m", 119, 1, "n", 119, 1, "a", 119, 1, "a",
                 97, 1, 97, 2>>),
     catch_b2t(<<131, 67, 0:32, 0, 97, 1, 119, 1, "n">>),
     catch_b2t(<<131, 67, 0:32, 0, 119, 1, "m", 107, 0, 1, 1>>),
     catch_b2t(<<131, 67, 1:32, 0, 119, 1, "m", 119, 1, "n", 106, 97, 1>>),
     catch_b2t(<<131, 67, 1:32, 0, 119, 1, "m", 119, 1, "n", 119, 1, "a">>),
     catch_b2t(<<131, 67, 0:32, 0, 119, 1, "m", 119, 1, "n">>),
     catch_b2t(<<131, 67, 0:32, 0, 88, 119, 3, "x@y", 0:96, 119, 1, "r">>),
     catch_b2t(<<131, 67, 2:32, 0, 119, 1, "m", 119, 1, "n", 119, 1, "a", 119, 1, "a",
                 88, 119, 3, "x@y", 0:96, 97, 1>>),
     catch_b2t(<<131, 67, 0:32, 0, 119, 1, "m", 119, 13, "nr_ext_absent">>, [safe])].

catch_b2t(B) ->
    catch_b2t(B, []).

catch_b2t(B, Opts) ->
    try binary_to_term(B, Opts) catch error:R -> {error, R} end.

%% Return the ~w print of results/0, as nr_bif:show/0 does.
show() ->
    persistent_term:put({erl_features, keywords}, ['maybe', 'else']),
    persistent_term:put({erl_features, init_done}, true),
    unicode:characters_to_binary(io_lib:format("~w~n", [results()])).

id(Term) ->
    Term.
