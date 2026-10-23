-module(nr_probe).
-export([results/0, show/0, id/1]).
-compile([{nowarn_deprecated_function, [{erlang, phash, 2}]}]).

-record #point{x = 0, y}.
-record #empty{}.

%% Return the observable results of native records: the instructions, the
%% is_record BIFs, badarg of tuple BIFs, term order, equality, hashes, and
%% flat size. No result holds a record, so the print is the same before
%% records print.
results() ->
    P = #point{x = id(1), y = id(2)},
    #point{x = X, y = Y} = P,
    D = #point{y = id(5)},
    U = P#point{x = id(9)},
    E = #empty{},
    Pub = nr_def:pub(id(5)),
    Priv = nr_def:priv(),
    [{X, Y}, fields(D), fields(U),
     catch_error(fun() -> (id({point, 1, 2}))#point{x = 3} end),
     catch_error(fun() -> (id(Pub))#point{x = 3} end),
     P#point.y, catch_error(fun() -> (id(Pub))#point.y end),
     catch_error(fun() -> (id(7))#point.y end),
     (id(P))#_.x, catch_error(fun() -> (id(P))#_.zz end),
     guard(P), guard(Pub), guard({point, 1, 2}), guard(E),
     [is_record(Term) || Term <- [P, E, {point, 1, 2}, #{}]],
     bif(is_record, [P]), bif(is_record, [{point, 1, 2}]),
     bif(is_record, [P, point]), bif(is_record, [P, empty]),
     bif(is_record, [{point, 1, 2}, point]), bif(is_record, [P, 1]),
     bif(is_record, [P, nr_probe, point]), bif(is_record, [P, nr_def, point]),
     bif(is_record, [P, point, 3]), bif(is_record, [{point, 1, 2}, point, 3]),
     bif(is_record, [P, point, 1.0]), bif(is_record, [{point, 1}, nr_probe, point]),
     bif(element, [1, P]), bif(tuple_size, [P]),
     bif(is_tuple, [P]), bif(is_map, [P]),
     P < U, U < P, P < #point{x = 1, y = 3}, #point{x = 1, y = 3} < #point{x = 2, y = 1},
     {a, b, c} < P, P < #{}, P < Pub, Pub < P, E < P, Priv < Pub, P < E,
     P == #point{x = 1.0, y = 2}, P =:= #point{x = 1.0, y = 2},
     P == {point, 1, 2}, P =:= P#point{x = id(1)}, E =:= #empty{},
     erlang:phash2(P), erlang:phash2(E), erlang:phash2(Priv),
     erlang:phash(P, 1 bsl 32), erlang:phash(E, 1 bsl 32), erlang:phash(Priv, 1 bsl 32),
     erts_debug:flat_size(P), erts_debug:flat_size(E), erts_debug:flat_size({P, P}),
     pdict([P, E, Priv, U, D, #point{x = 2, y = 3}, 1, {3}, [4], 2.5, P#point{y = 7}]),
     remote(Pub, Priv)].

%% The remote record operations of OTP 29 (`erl_record.c:344-568`).
remote(Pub, Priv) ->
    #nr_def:pub{a = A, b = B} = Pub,
    [{A, B}, Pub#nr_def:pub.b,
     catch_error(fun() -> #nr_def:priv{} end),
     catch_error(fun() -> #nr_def:pub{} end),
     catch_error(fun() -> #nr_def:pub{a = 1, zz = id(2)} end),
     catch_error(fun() -> #nr_none:pub{} end),
     pub_fields(#nr_def:pub{a = id(3)}),
     pub_fields(Pub#nr_def:pub{b = id(4)}),
     catch_error(fun() -> (id(Pub))#nr_def:pub{zz = 1} end),
     catch_error(fun() -> (id(Priv))#nr_def:pub{a = 1} end),
     catch_error(fun() -> (id(Priv))#_{z = 9} end),
     catch_error(fun() -> (id(Priv))#nr_def:priv.z end),
     catch_error(fun() -> (id(Priv))#nr_def:pub.a end),
     catch_error(fun() -> (id(Pub))#nr_def:pub.zz end),
     (id(Priv))#_.z, catch_error(fun() -> (id(Priv))#_.a end),
     pub_fields((id(Pub))#_{a = 8}),
     case id(Priv) of #nr_def:priv{z = Z} -> Z; _ -> nomatch end,
     case id(Pub) of #nr_def:pub{zz = Zz} -> Zz; _ -> nomatch end,
     case id(Pub) of #nr_def:pub{} -> pub; _ -> nomatch end,
     is_record(Pub, nr_def, pub), is_record(Priv, nr_def, pub)].

%% Return the values of a process dictionary with each key of Keys, in the
%% order of get/0. The order follows the internal hash of each key
%% (`erl_term_hashing.c:1850-1875`).
pdict(Keys) ->
    Self = self(),
    spawn(fun() -> put_all(Keys, 1), Self ! {pdict, [V || {_, V} <- get()]} end),
    receive {pdict, Values} -> Values end.

put_all([Key | Keys], N) ->
    put(Key, N),
    put_all(Keys, N + 1);
put_all([], _) ->
    ok.

fields(#point{x = X, y = Y}) ->
    {X, Y}.

pub_fields(#nr_def:pub{a = A, b = B}) ->
    {A, B}.

guard(R) when is_record(R, nr_probe, point) -> point;
guard(R) when is_record(R) -> record;
guard(_) -> other.

bif(Name, Args) ->
    catch_error(fun() -> apply(erlang, id(Name), id(Args)) end).

%% Return the error reason of Fun. A record in the reason gives its name,
%% so the print holds no record. The order of the keys of #nr_def:pub{}
%% follows the atom indexes of a and b, which change from run to run of
%% ERTS, so the probe prints no hash of it.
catch_error(Fun) ->
    try Fun() of
        Value -> Value
    catch
        error:{badrecord, R} when is_record(R, nr_def, pub) -> {error, {badrecord, pub}};
        error:{badrecord, R} when is_record(R, nr_def, priv) -> {error, {badrecord, priv}};
        error:Reason -> {error, Reason}
    end.

%% Exit with the result of results/0, so the host reads it.
show() ->
    exit(results()).

id(Term) ->
    Term.
