-module(nr_bif).
-export([results/0, show/0, id/1]).

-record #point{x = 0, y}.
-record #empty{}.

%% Return the results of the records module BIFs and the io_lib print of
%% native records.
results() ->
    P = #point{x = id(1), y = id(2)},
    E = #empty{},
    Pub = nr_def:pub(id(5)),
    Priv = nr_def:priv(),
    T = {point, 1, 2},
    Opts = #{is_exported => false},
    Big = maps:from_list([{list_to_atom([$k | integer_to_list(I)]), I} || I <- lists:seq(1, 33)]),
    [bif(get_module, [P]), bif(get_module, [T]),
     bif(get_name, [Pub]), bif(get_name, [T]),
     bif(get, [y, P]), bif(get, [zz, P]), bif(get, [1, P]), bif(get, [x, T]),
     bif(get_field_names, [P]), bif(get_field_names, [Pub]), bif(get_field_names, [E]),
     bif(get_field_names, [T]),
     bif(is_exported, [P]), bif(is_exported, [Pub]), bif(is_exported, [7]),
     bif(get_definition, [nr_bif, point]), bif(get_definition, [nr_def, pub]),
     bif(get_definition, [nr_def, priv]), bif(get_definition, [nr_bif, empty]),
     bif(get_definition, [nr_bif, none]), bif(get_definition, [nr_none, point]),
     bif(get_definition, [1, point]), bif(get_definition, [nr_bif, "point"]),
     same(bif(create, [nr_def, pub, [{b, 1}, {a, 5}], #{is_exported => true}]), Pub),
     same(bif(create, [nr_def, pub, [{a, 5}, {b, 1}], #{is_exported => true}]), Pub),
     same(bif(create, [nr_bif, point, [{x, 1}, {y, 2}], Opts]), P),
     same(bif(create, [nr_bif, empty, [], Opts]), E),
     fields(bif(create, [nr_bif, other, [{y, 2}, {x, 1}], Opts])),
     bif(create, [1, point, [], Opts]), bif(create, [nr_bif, "p", [], Opts]),
     bif(create, [nr_bif, point, [], [is_exported]]),
     bif(create, [nr_bif, point, [], #{}]),
     bif(create, [nr_bif, point, [], #{is_exported => true, a => 1}]),
     bif(create, [nr_bif, point, [], #{a => 1}]),
     bif(create, [nr_bif, point, [], #{is_exported => yes}]),
     bif(create, [nr_bif, point, [{x, 1} | y], Opts]),
     bif(create, [nr_bif, point, [x], Opts]),
     bif(create, [nr_bif, point, [{x, 1, 2}], Opts]),
     bif(create, [nr_bif, point, [{x, 1}, x, {"y", 2}], Opts]),
     bif(create, [nr_bif, point, [{x, 1}, {"y", 2}, x], Opts]),
     bif(create, [nr_bif, point, [{x, 1}, {x, 2}], Opts]),
     fields(bif(update, [P, nr_bif, point, #{x => 7}])),
     fields(bif(update, [P, any, '_', #{y => 8, x => 9}])),
     bif(update, [P, nr_bif, point, #{}]) =:= P,
     bif(update, [T, nr_bif, point, #{}]),
     bif(update, [P, nr_def, point, #{}]), bif(update, [P, nr_bif, pub, #{}]),
     bif(update, [P, 1, point, #{}]), bif(update, [P, nr_bif, 2, #{}]),
     bif(update, [P, nr_bif, point, [x]]), bif(update, [P, 1, '_', x]),
     bif(update, [P, nr_bif, point, #{zz => 1}]),
     bif(update, [P, nr_bif, point, #{x => 1, zz => 1}]),
     bif(update, [P, nr_bif, point, #{1 => 1, x => 1}]),
     bif(update, [P, nr_bif, point, #{{t} => 1, y => 1}]),
     bif(update, [P, nr_bif, point, Big]),
     bif(update, [E, nr_bif, empty, #{x => 1}]),
     fmt("~p ~w ~p ~w ~p", [P, P, E, Pub, [Priv, {P}]]),
     fmt("~P ~W", [#point{x = [1, 2, 3], y = Pub}, 4, P, 2]),
     fmt("~p", [#point{x = lists:seq(1, 30), y = lists:duplicate(4, Pub)}]),
     fmt("~tp ~ts", [bif(create, [nr_bif, 'ä', [{'ö', <<"ü"/utf8>>}], Opts]), "x"])].

fmt(Format, Args) -> unicode:characters_to_binary(io_lib:format(Format, Args)).

same({error, _} = Error, _) -> Error;
same(R, Expected) -> {R =:= Expected, R == Expected}.

fields({error, _} = Error) -> Error;
fields(R) -> [{F, records:get(F, R)} || F <- records:get_field_names(R)].

bif(Name, Args) ->
    try apply(records, Name, Args) catch error:Reason -> {error, Reason} end.

%% Return the ~w text of results/0 with the erl_features state that the
%% boot of the pinned ERTS stores (erl_features.erl:474-500), as no init process runs.
show() ->
    persistent_term:put({erl_features, keywords}, ['maybe', 'else']),
    persistent_term:put({erl_features, init_done}, true),
    unicode:characters_to_binary(io_lib:format("~w~n", [results()])).

id(X) -> X.
