-module(persistent_term_probe).
-export([main/0, put_term/3]).

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
