-module(root_exit_probe).
-export([terms/0, identifiers/0]).

%% Exit with one term of each kind that PBeam can represent, except pids and
%% references. Under erl -eval, init writes the reason with
%% erts_internal:term_to_string/2 in "Runtime terminating during boot (...)"
%% (init.erl:665-679, 710-712).
terms() ->
    exit({ok, 'Elixir.Hello', 'hello world', '', 'it\'s', 'a\nb', 'a\eb',
          '\\\f\t\r\b\v', '\d', 'end', '\x{80}', 'Ä', 'é', 'λ', '_x', aB_9,
          -7, 576460752303423488, 9223372036854775807,
          [], "abc", "q\"uo\n\t", [1, 2, 3], [65 | 66], [a | b], [128], [160, 255], [300],
          {}, {a, {b}},
          <<>>, <<"abc">>, <<"a\"b">>, <<1, 2, 200>>, <<"a\n">>,
          #{}, #{a => 1, "k" => [], 2 => b},
          #{1 => 1, 2 => 2, 3 => 3, 4 => 4, 5 => 5, 6 => 6, 7 => 7, 8 => 8, 9 => 9, 10 => 10, 11 => 11, 12 => 12, 13 => 13, 14 => 14, 15 => 15, 16 => 16, 17 => 17, 18 => 18, 19 => 19, 20 => 20, 21 => 21, 22 => 22, 23 => 23, 24 => 24, 25 => 25, 26 => 26, 27 => 27, 28 => 28, 29 => 29, 30 => 30, 31 => 31, 32 => 32, 33 => 33},
          fun lists:map/2, fun() -> ok end, fun(X) -> {X} end}).

%% Exit with the pid of the root process and a new reference.
identifiers() ->
    exit({self(), make_ref()}).
