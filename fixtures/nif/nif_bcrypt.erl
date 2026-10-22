-module(nif_bcrypt).
-export([main/0]).

%% Call the three NIFs of 'Elixir.Bcrypt.Base' of bcrypt_elixir and print
%% the results. The on_load function of the module loads bcrypt_nif.so from
%% the priv directory of bcrypt_elixir. hash_nif/2 and checkpass_nif/2 are
%% dirty CPU NIFs.
main() -> io:format("~p~n", [results()]).

results() ->
    B = 'Elixir.Bcrypt.Base',
    Random = list_to_binary(lists:seq(1, 16)),
    Salt = B:gensalt_nif(Random, 4, $b),
    Hash = B:hash_nif("password", binary_to_list(Salt)),
    {Salt,
     B:gensalt_nif(Random, 40, $a),
     B:gensalt_nif(Random, 1, $b),
     Hash,
     B:hash_nif(lists:duplicate(100, $a), binary_to_list(Salt)),
     B:hash_nif("", binary_to_list(Salt)),
     B:checkpass_nif("password", binary_to_list(Hash)),
     B:checkpass_nif("wrong", binary_to_list(Hash)),
     B:checkpass_nif("password", "not a hash"),
     caught(fun() -> B:gensalt_nif(<<1>>, 4, $b) end),
     caught(fun() -> B:gensalt_nif(Random, -1, $b) end),
     caught(fun() -> B:gensalt_nif(a, 4, $b) end),
     caught(fun() -> B:hash_nif(<<"password">>, binary_to_list(Salt)) end),
     caught(fun() -> B:hash_nif("password", "$3b$04$abc") end),
     caught(fun() -> B:checkpass_nif([256], binary_to_list(Hash)) end),
     caught(fun() -> B:checkpass_nif("password", [$$ | a]) end)}.

%% Return the class, the reason, and the first stack frame of the exception
%% of Fun.
caught(Fun) ->
    try Fun() catch Class:Reason:Stack -> {Class, Reason, hd(Stack)} end.
