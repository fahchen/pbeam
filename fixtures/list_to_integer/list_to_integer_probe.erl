-module(list_to_integer_probe).
-export([main/0]).

%% erts_internal:list_to_integer/2 for each input shape of
%% erts_internal_list_to_integer_2 (big.c:4258-4354), and
%% erlang:list_to_integer/1,2 of erlang.beam, which call it
%% (erlang.erl:4111-4131, 4165-4182).
main() ->
    Internal = [erts_internal:list_to_integer(String, Base)
                || {String, Base} <- [{"", 10}, {a, 10}, {<<"1">>, 10}, {"12", 10},
                                      {"-12a", 10}, {"+7", 10}, {"-", 10}, {"+", 10},
                                      {"000", 10}, {"-00a", 10}, {"0a", 10}, {"0a", 16},
                                      {"Zz", 36}, {"12", 2}, {"1", 1}, {"1", 37},
                                      {"1", ten}, {"1", -10}, {[$1 + 256], 10},
                                      {[-207], 10}, {[$- + 256, $1], 10},
                                      {"12345678901234567", 10}, {"123456789012345678", 10},
                                      {"-0000000000000000000000001", 10}, {[$1 | foo], 10},
                                      {[$- | foo], 10}, {["1"], 10}, {[$1, "2"], 10},
                                      {[$0, a], 10}, {[$0 | b], 10}, {[$0, $0 | b], 10},
                                      {[$1, $0 | b], 10}, {[a], 10}]],
    External = [try list_to_integer(String) catch Class:Reason -> {Class, Reason} end
                || String <- ["42", "4x", foo]]
        ++ [try list_to_integer(String, Base) catch Class:Reason -> {Class, Reason} end
            || {String, Base} <- [{"-7f", 16}, {"1", 99}]],
    {Internal, External}.
