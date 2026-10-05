-module(persistent_term_probe).
-export([main/0, put_term/3, table/0]).

main() ->
    Key = {?MODULE, key},
    Missing = persistent_term:get(Key, default),
    First = persistent_term:put(Key, 1),
    One = persistent_term:get(Key, default),
    Same = persistent_term:put(Key, 1),
    spawn(?MODULE, put_term, [self(), Key, {value, [a, <<"b">>]}]),
    Replaced = receive {put, Result} -> Result end,
    Value = persistent_term:get(Key, default),
    Other = persistent_term:get({?MODULE, other}, undefined),
    {Missing, First, One, Same, Replaced, Value, Other}.

put_term(Parent, Key, Value) ->
    Parent ! {put, persistent_term:put(Key, Value)}.

table() ->
    Empty = persistent_term:get(),
    ok = persistent_term:put(true, 1),
    ok = persistent_term:put(39, 2),
    ok = persistent_term:put(ok, 3),
    ok = persistent_term:put(7, 4),
    Small = persistent_term:get(),
    ok = persistent_term:put({ok}, 5),
    ok = persistent_term:put(<<"0123456789abcdefg">>, 6),
    ok = persistent_term:put("abc", 7),
    ok = persistent_term:put(1 bsl 60, 8),
    Grown = persistent_term:get(),
    ok = persistent_term:put([], 9),
    ok = persistent_term:put([ok, {}], 10),
    ok = persistent_term:put(-1, 11),
    ok = persistent_term:put(#{ok => [], 1 => <<>>}, 12),
    ok = persistent_term:put(#{1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7, 8 => 8, 9 => 9, 10 => 10, 11 => 11,
                               12 => 12, 13 => 13, 14 => 14, 15 => 15, 16 => 16, 17 => 17, 18 => 18, 19 => 19, 20 => 20, 21 => 21, 22 => 22,
                               23 => 23, 24 => 24, 25 => 25, 26 => 26, 27 => 27, 28 => 28, 29 => 29, 30 => 30, 31 => 31, 32 => 32, 33 => 33}, 13),
    ok = persistent_term:put(ok, 30),
    ok = persistent_term:put(true, 1),
    {Empty, Small, Grown, persistent_term:get()}.
