-module(term_to_string_probe).
-export([cases/0]).

%% erts_internal:term_to_string/2 prints with "%.*T" and the precision
%% Limit (bif.c:4156-4190, erl_printf_term.c:352-824). Each term takes 1,
%% an atom also its name length, a string character 1 before it prints, and
%% a binary byte 1 after it prints. "... " follows when the count is at
%% most 0. Limit is 2..INT_MAX-1 or undefined, else badarg.
cases() ->
    [result(T, L)
     || {T, L} <- [{ab, 2}, {{a, "xyz", <<1, 2, 3>>}, 5},
                   {{a, "xyz", <<1, 2, 3>>}, 8}, {[1, 2, 3], 3},
                   {[a | b], 100}, {<<"abc">>, 2}, {<<1, 2, 3:4>>, 3},
                   {<<1, 2, 3:4>>, 4}, {fun lists:map/2, 4},
                   {fun lists:map/2, 20}, {"ab\n", 3}, {#{a => 1}, 3},
                   {#{1 => a, 2 => bb}, 4}, {#{1 => a, 2 => bb}, 7},
                   {x, 1}, {x, 2147483647}, {x, 2.0}, {x, undefined}]].

result(T, L) ->
    try
        erts_internal:term_to_string(T, L)
    catch
        error:badarg -> badarg
    end.
