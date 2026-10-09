-module(integer_to_binary_probe).
-export([main/0, id/1]).

%% erlang:integer_to_binary/1, as Integer.to_string/1 of Elixir runs it.
%% 2^59 - 1 and -2^59 are the limits of an ERTS small integer. The
%% integers next to them, 2^63 - 1, and -2^63 are ERTS bignums.
main() ->
    [integer_to_binary(id(I))
     || I <- [0, 7, -15, 1000000, 576460752303423487, -576460752303423488,
              id(576460752303423487) + 1, id(-576460752303423488) - 1,
              id(9223372036854775807), id(-9223372036854775808)]]
        ++ [try integer_to_binary(id(T)) catch error:R -> {error, R} end
            || T <- [a, <<"1">>, "1", [], {1}]].

%% Return the argument, so that the compiler does not fold a constant.
id(X) -> X.
