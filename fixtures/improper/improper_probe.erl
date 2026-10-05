-module(improper_probe).
-export([literals/0, instructions/0, list_bifs/0, iodata/0, cons/2, id/1, ge/2, lt/2]).

%% Literal improper lists: exact equality, the term order, and map keys.
literals() ->
    Pairs = [
        {[1|x], [1,2]}, {[1|<<>>], [1,2]}, {[1,2|a], [1,2|b]}, {[1|a], [1]},
        {[1|2], [1|x]}, {[], [1|x]}, {{a}, [1|x]}, {[1|x], [1|x]}, {[1|{}], [1]}
    ],
    Flat = ?MODULE:id(#{[1|x] => a, [1] => b, x => c, [[1|2]|3] => d}),
    Hash = ?MODULE:id(#{[1|2] => v, [1,2|3] => v, [[1|2]|3] => v, [true|false] => v, [256|1] => v, [[1]|2] => v, [[]|-1] => v, 1 => v, 2 => v, 3 => v, 4 => v, 5 => v, 6 => v, 7 => v, 8 => v, 9 => v, 10 => v, 11 => v, 12 => v, 13 => v, 14 => v, 15 => v, 16 => v, 17 => v, 18 => v, 19 => v, 20 => v, 21 => v, 22 => v, 23 => v, 24 => v, 25 => v, 26 => v}),
    {?MODULE:id([a|b]), ?MODULE:id({[1,2|{t}]}),
     [?MODULE:ge(A, B) || {A, B} <- Pairs], [?MODULE:lt(A, B) || {A, B} <- Pairs],
     [A =:= B || {A, B} <- Pairs], [A >= B || {A, B} <- Pairs],
     maps:keys(Flat), map_get([[1|2]|3], Flat), maps:keys(Hash), map_get([1,2|3], Hash)}.

%% put_list, get_list, get_hd, get_tl, is_list, and is_nonempty_list.
instructions() ->
    Ref = make_ref(),
    Tag = ?MODULE:cons(alias, Ref),
    self() ! {Tag, reply},
    Reply = receive {[alias | Mref], R} when Mref =:= Ref -> R end,
    Imp = ?MODULE:cons(1, x),
    Two = ?MODULE:cons(0, Imp),
    Nested = ?MODULE:cons(?MODULE:cons(1, 2), 3),
    [Reply, Imp, Two, Nested, head(Two), tail(Two), tail(Imp), split(Nested),
     kind(Imp), kind(?MODULE:id([])), kind(?MODULE:id(x)), ?MODULE:cons(Two, []), Tag =:= [alias | Ref]].

head([H | _]) -> H.
tail([_ | T]) -> T.
split([[A | B] | C]) -> {A, B, C}.
kind(L) when is_list(L), L =/= [] -> nonempty;
kind([]) -> nil;
kind(_) -> other.

%% The list BIFs with improper lists.
list_bifs() ->
    X = ?MODULE:id(x), One = ?MODULE:id([1]), Imp = ?MODULE:id([1|x]),
    Long = dup(300, 97) ++ X,
    [t(fun() -> One ++ X end), t(fun() -> One ++ ?MODULE:id([2|x]) end), t(fun() -> Imp ++ [2] end),
     t(fun() -> ?MODULE:id([]) ++ X end),
     t(fun() -> tl(Imp) end), t(fun() -> tl(?MODULE:id([1,2|x])) end), t(fun() -> length(Imp) end),
     t(fun() -> lists:reverse(?MODULE:id([a]), X) end), t(fun() -> lists:reverse(?MODULE:id([a,b]), ?MODULE:id([c|x])) end),
     t(fun() -> lists:reverse(Imp, []) end), t(fun() -> lists:reverse(?MODULE:id([]), Imp) end),
     t(fun() -> lists:member(1, Imp) end), t(fun() -> lists:member(2, Imp) end),
     t(fun() -> lists:keyfind(a, 1, ?MODULE:id([{a}|x])) end), t(fun() -> lists:keyfind(b, 1, ?MODULE:id([{a}|x])) end),
     t(fun() -> lists:keymember(a, 1, ?MODULE:id([{a}|x])) end), t(fun() -> lists:keymember(b, 1, ?MODULE:id([{a}|x])) end),
     t(fun() -> list_to_atom(Long) end), t(fun() -> list_to_atom(?MODULE:id([97|x])) end),
     t(fun() -> apply(?MODULE, id, Imp) end), t(fun() -> apply(fun ?MODULE:id/1, Imp) end),
     t(fun() -> spawn(?MODULE, id, Imp) end), t(fun() -> spawn(?MODULE, id, X) end), t(fun() -> spawn_link(?MODULE, id, Imp) end),
     t(fun() -> spawn_opt(?MODULE, id, [1], ?MODULE:id([link|x])) end),
     t(fun() -> erlang:demonitor(make_ref(), ?MODULE:id([flush|x])) end),
     t(fun() -> erlang:demonitor(make_ref(), X) end),
     t(fun() -> erlang:demonitor(make_ref(), ?MODULE:id([bogus])) end),
     t(fun() -> erlang:demonitor(make_ref(), ?MODULE:id([info])) end),
     t(fun() -> erlang:demonitor(erlang:monitor(process, spawn(?MODULE, id, [1])), ?MODULE:id([info])) end),
     t(fun() -> process_info(self(), ?MODULE:id([registered_name|x])) end),
     t(fun() -> erlang:cancel_timer(make_ref(), ?MODULE:id([{async, false}|x])) end),
     t(fun() -> ets:new(t, ?MODULE:id([set|x])) end),
     t(fun() -> ets:insert(ets:new(t, []), ?MODULE:id([{a}|x])) end),
     t(fun() -> erlang:demonitor(X, ?MODULE:id([flush|x])) end), t(fun() -> erlang:demonitor(X, ?MODULE:id([])) end),
     t(fun() -> spawn(?MODULE:id(1), id, [1]) end), t(fun() -> spawn_link(?MODULE, ?MODULE:id(1), [1]) end),
     t(fun() -> spawn_opt(?MODULE:id(1), id, [1], ?MODULE:id([link|x])) end),
     t(fun() -> spawn_opt(?MODULE:id(1), id, [1], ?MODULE:id([])) end)].

%% The iodata and chardata BIFs with improper lists.
iodata() ->
    [t(fun() -> erlang:iolist_to_iovec(?MODULE:id([97|<<"b">>])) end),
     t(fun() -> erlang:iolist_to_iovec(?MODULE:id([[97|<<"b">>], 99])) end),
     t(fun() -> erlang:iolist_to_iovec(?MODULE:id([97|x])) end),
     t(fun() -> element(1, re:compile(?MODULE:id([97|<<"b">>]), [])) end),
     t(fun() -> re:compile(?MODULE:id([97|x]), []) end),
     t(fun() -> re:compile(<<"a">>, ?MODULE:id([unicode|x])) end),
     t(fun() -> unicode:characters_to_binary(?MODULE:id([97|<<"b">>]), unicode) end),
     t(fun() -> unicode:characters_to_binary(?MODULE:id([[<<195>>|<<169>>]]), unicode) end),
     t(fun() -> unicode:characters_to_binary(?MODULE:id([97|x]), unicode) end),
     t(fun() -> unicode:characters_to_binary(?MODULE:id([-1|x]), unicode) end),
     t(fun() -> unicode:characters_to_binary(?MODULE:id([foo]), unicode) end)].

%% Run a fun and return its result, or `{Class, Reason}` of its exception.
t(Fun) -> try Fun() catch Class:Reason -> {Class, Reason} end.

%% Return a list of N copies of E. A list that the compiler cannot fold
%% makes `Long` a run-time term.
dup(0, _) -> [];
dup(N, E) -> [E | dup(N - 1, E)].

%% Return a list cell with a head and a tail. The tail of `gen:do_call/4`
%% is a reference, so `put_list` gets a register tail.
cons(Head, Tail) -> [Head | Tail].

%% Return the argument. A call through ?MODULE stops the compiler from
%% folding a constant term.
id(Term) -> Term.

%% The guard `A >= B` compiles to `is_ge`.
ge(A, B) when A >= B -> ge;
ge(_, _) -> lt.

%% The guard `A < B` compiles to `is_lt`.
lt(A, B) when A < B -> lt;
lt(_, _) -> ge.
