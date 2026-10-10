-module(inspect_bif_probe).
-export([main/0, id/1]).

%% The BIFs that IO.inspect({:ok, 1}) of Elixir calls:
%% erlang:tuple_to_list/1 (Inspect.Tuple) and erlang:integer_to_binary/2
%% (Inspect.Integer). The result has one list for each part.
%% 2^59 - 1 and -2^59 are the limits of an ERTS small integer. A literal
%% integer outside the 32-bit range is not a PBeam literal, so the
%% integer operands of id/1 give these limits, and the arithmetic builds
%% each bignum.
main() ->
    MaxSmall = id(576460752303423487),
    MinSmall = id(-576460752303423488),
    Big = MaxSmall + 1,
    Max = MaxSmall * 16 + 15,
    Min = MinSmall * 16,
    [[tuple_to_list(id(T)) || T <- [{}, {ok, 1}, {a, {b}, [c], <<"d">>}]],
     [try tuple_to_list(id(T)) catch error:R -> {error, R} end
      || T <- [[], [ok, 1], a, 1, <<>>]],
     [integer_to_binary(id(I), id(B))
      || {I, B} <- [{0, 2}, {1, 10}, {-15, 10}, {255, 16}, {-255, 16}, {35, 36}, {36, 36},
                    {5, 2}, {MaxSmall, 36}, {MinSmall, 8},
                    {Big, 2}, {-Big, 16}, {Max, 2}, {Max, 36}, {Min, 16}, {Min, 10}]],
     [try integer_to_binary(id(I), id(B)) catch error:R -> {error, R} end
      || {I, B} <- [{a, 10}, {<<"1">>, 10}, {1, 1}, {1, 37}, {1, 0}, {1, -10}, {1, a},
                    {1, Big}, {Big, 1}]]].

%% Return the argument, so that the compiler does not fold a constant.
id(X) -> X.
