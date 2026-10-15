-module(process_dict_probe).
-export([small/0, large/1]).

%% Put a few keys of each shape, replace two values, and erase keys.
small() ->
    erase(),
    Empty = {get(), get_keys(), get_keys(0), erase(ok), erase()},
    put_atoms([ok, error, true, false, undefined, badarg]),
    put_range(1, 12, 0),
    Boxed = put(key(3), {boxed}),
    Immediate = put(key(5), 7),
    Pairs = get(),
    Keys = get_keys(),
    BoxedKeys = get_keys({boxed}),
    Erased = erase(key(3)),
    Missing = erase(missing),
    {Empty, Boxed, Immediate, Pairs, Keys, BoxedKeys, Erased, Missing,
     get(), get_keys(0), erase(), get(), get_keys()}.

%% Grow the table to more than N buckets, replace and erase keys, shrink the
%% table with erases of present and missing keys, and grow it again.
large(N) ->
    erase(),
    put_range(1, N, 0),
    Grown = get_keys(),
    replace(1, N),
    Replaced = get(),
    erase_every(3, 1, N),
    AfterErase = get(),
    Tagged = get_keys(tagged),
    erase_range(1, N - N div 10),
    Shrunk = get(),
    erase_missing(20),
    Missing = get_keys(),
    put_range(N + 1, N + N div 4, 1),
    Regrown = get(),
    {Grown, Replaced, AfterErase, Tagged, Shrunk, Missing, Regrown, erase(), get()}.

%% Return a key of one of six shapes: an integer, a negative integer, a
%% tuple, a list, a binary, or a big integer.
key(I) ->
    case I rem 6 of
        0 -> I;
        1 -> -I;
        2 -> {I, I * I};
        3 -> [I, I + 1];
        4 -> <<0, I:16>>;
        5 -> I + (1 bsl 62)
    end.

put_atoms([]) -> ok;
put_atoms([Atom | Rest]) ->
    put(Atom, 0),
    put_atoms(Rest).

put_range(I, N, _) when I > N -> ok;
put_range(I, N, Value) ->
    put(key(I), Value),
    put_range(I + 1, N, Value).

%% Give every fifth key a boxed value, every seventh key an immediate value,
%% and every eleventh key the value tagged.
replace(I, N) when I > N -> ok;
replace(I, N) ->
    if
        I rem 5 =:= 0 -> put(key(I), {I});
        I rem 7 =:= 0 -> put(key(I), I);
        I rem 11 =:= 0 -> put(key(I), tagged);
        true -> ok
    end,
    replace(I + 1, N).

erase_every(_, I, N) when I > N -> ok;
erase_every(Step, I, N) ->
    erase(key(I)),
    erase_every(Step, I + Step, N).

erase_range(I, N) when I > N -> ok;
erase_range(I, N) ->
    erase(key(I)),
    erase_range(I + 1, N).

erase_missing(0) -> ok;
erase_missing(I) ->
    erase({missing, I}),
    erase_missing(I - 1).
