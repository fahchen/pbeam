-module(float_probe).
-export([literals/0, arith/0, registers/2, register_errors/0, compare/0,
         keys/0, select/0, to_text/0, text_errors/0, format/0, id/1, show/1]).

%% Return the term unchanged. The calls through id/1 keep the compiler from
%% folding the operations at compile time.
id(Term) -> Term.

%% Return the text of the result of a function of this module, as `~w`
%% writes a term of numbers, atoms, binaries, tuples, and lists. The text
%% of a float is its shortest text. The function uses no io_lib code, which
%% prints an atom with erl_scan and the init arguments.
show(Name) ->
    iolist_to_binary(text(?MODULE:Name())).

text(Term) when is_float(Term) -> float_to_binary(Term, [short]);
text(Term) when is_integer(Term) -> integer_to_binary(Term);
text(Term) when is_atom(Term) -> atom_to_binary(Term);
text(Term) when is_binary(Term) -> [<<"<<\"">>, Term, <<"\">>">>];
text(Term) when is_tuple(Term) -> [${, join(tuple_to_list(Term)), $}];
text(Term) when is_list(Term) -> [$[, join(Term), $]].

join(Terms) -> lists:join($,, [text(Term) || Term <- Terms]).

%% Float literals from the literal table (`NEW_FLOAT_EXT`).
literals() ->
    [1.5, -0.0, 0.0, 0.1, 1.0e300, 5.0e-324, -2.5e-10, 123456789.0,
     {2.0, [3.5]}].

%% `gc_bif` arithmetic with float operands, the `/` operator, and the
%% guard BIFs float/1 and abs/1. An overflow or a term that is not a
%% number raises badarith.
arith() ->
    [id(1.5) + id(2), id(1) + id(2.25), id(0.1) + id(0.2), id(5) - id(0.5),
     id(2.0) * id(3), id(7) / id(2), id(1) / id(4), id(-1.5) * id(0.0),
     -id(2.5), -id(0.0), id(1.0) - id(1.0), float(id(3)), float(id(-0.5)),
     float(id(9007199254740993)), abs(id(-2.5)), abs(id(-0.0)), abs(id(-3)),
     try id(1.0e308) + id(1.0e308) catch error:R1 -> R1 end,
     try id(1.0e308) * id(10) catch error:R2 -> R2 end,
     try id(1) / id(0) catch error:R3 -> R3 end,
     try id(0.0) / id(0.0) catch error:R4 -> R4 end,
     try id(1.5) + id(a) catch error:R5 -> R5 end,
     try float(id(a)) catch error:R6 -> R6 end,
     try abs(id(a)) catch error:R7 -> R7 end,
     apply(erlang, id('/'), [id(7), id(2)]),
     apply(erlang, id(is_float), [id(1.5)]),
     apply(erlang, id(is_float), [id(1)])].

%% Arithmetic on two floats in float registers: `fmove`, `fadd`, `fsub`,
%% `fmul`, `fdiv`, and `fnegate`.
registers(X, Y) when is_float(X), is_float(Y) ->
    [X + Y, X - Y, X * Y, X / Y, -X, (X * Y + 1.0) / (Y - X)].

%% The float-register instructions raise badarith on an overflow and on a
%% result that is not a number.
register_errors() ->
    [try registers(id(1.0e308), id(1.0e308)) catch error:R1 -> R1 end,
     try registers(id(1.0), id(0.0)) catch error:R2 -> R2 end,
     try registers(id(0.0), id(-0.0)) catch error:R3 -> R3 end].

%% Arithmetic and exact comparisons of integers and floats, and the term
%% order of a sort.
compare() ->
    [id(1) == id(1.0), id(1) =:= id(1.0), id(1) /= id(1.0), id(1) =/= id(1.0),
     id(1) < id(1.5), id(2) > id(1.5), id(1.0) =< id(1), id(1.0) >= id(1),
     id(0.0) == id(-0.0), id(0.0) =:= id(-0.0), id(-0.0) < id(0.0),
     id(9007199254740993) > id(9007199254740992.0),
     id(-1.0e19) < id(-9223372036854775807),
     id(9223372036854775807) < id(9.3e18),
     id(1.5) < id(a), id(#{a => 1}) == id(#{a => 1.0}),
     id(#{a => 1}) =:= id(#{a => 1.0}), id({1, 2.0}) == id({1.0, 2}),
     id([1]) == id([1.0]), id(#{1 => a}) == id(#{1.0 => a}),
     id(#{1 => a}) < id(#{1.0 => a}),
     lists:sort([id(1.0), 1, id(-0.0), id(0.0), 0, 2.5, a, -3]),
     lists:usort([id(1.0), 1, id(0.0), id(-0.0)]),
     lists:keyfind(id(1), 1, [{1.0, a}]),
     lists:keyfind(id(9007199254740993), 1, [{9007199254740992.0, b}]),
     lists:keyfind(id(1.0), 1, [{1, c}]),
     lists:keyfind(id(9223372036854775807), 1, [{9223372036854775808.0, d}]),
     id([1.0, 1]) -- id([1])].

%% Float map keys and ETS keys: `1` and `1.0` are two keys, and `0.0` and
%% `-0.0` are two keys.
keys() ->
    Small = #{id(1) => a, id(1.0) => b, id(-0.0) => c, id(0.0) => d},
    Big = maps:from_list([{float(I), I} || I <- lists:seq(1, 40)]),
    T = ets:new(float_probe, [set]),
    true = ets:insert(T, [{1.0, a}, {1, b}, {-0.0, c}, {0.0, d}]),
    Ets = [ets:lookup(T, 1.0), ets:lookup(T, 1), ets:lookup(T, -0.0),
           ets:lookup(T, 0.0), ets:member(T, 2.0)],
    [maps:to_list(Small), maps:get(1.0, Small), maps:get(-0.0, Small),
     maps:keys(Big), maps:get(7.0, Big), maps:is_key(7, Big), Ets].

%% float_to_list/1,2 and float_to_binary/1,2 with each option.
to_text() ->
    Floats = [1.5, 0.1, -0.0, 0.0, 1.0e20, 1.0e21, 123456789012345680.0,
              1.0e-5, 1.0e-4, 5.0e-324, 1.7976931348623157e308, 100.0,
              123456.0, 1.0e15, 1.0e16, 9007199254740993.0, -2.5e-10,
              2.5, 0.125, 9.5367431640625e-7, 1.0e300, 0.3, 2/3,
              2012773988401152.25, 11697403767521.5625, 0.999],
    Options = [[], [short], [compact], [{decimals, 0}], [{decimals, 2}],
               [{decimals, 3}, compact], [{decimals, 19}], [{decimals, 30}],
               [{scientific, 0}], [{scientific, 3}], [{scientific, -1}],
               [{decimals, id(4294967297)}], [short, {decimals, 2}]],
    [[float_to_list(F) || F <- Floats], [float_to_binary(F) || F <- Floats]
     | [[text(F, O) || F <- Floats] || O <- Options]].

text(F, O) ->
    try float_to_binary(F, O) catch error:R -> R end.

%% A `select_val` of more than 32 choices takes the ordered form, which
%% binary-searches the sorted choices. ERTS matches each choice exactly, as
%% `=:=` does: an integer never matches a float choice, a float never
%% matches an integer choice, and `-0.0` does not match a `+0.0` choice.
select() ->
    [[int_choice(id(V)) || V <- [1.0, 5.0, 5, 0, 0.0, -0.0, 40.0]],
     [float_choice(id(V)) || V <- [1, 7, 7.0, 0, 0.0, -0.0, 40]]].

int_choice(V) ->
    case V of
        0 -> i0;
        1 -> i1;
        2 -> i2;
        3 -> i3;
        4 -> i4;
        5 -> i5;
        6 -> i6;
        7 -> i7;
        8 -> i8;
        9 -> i9;
        10 -> i10;
        11 -> i11;
        12 -> i12;
        13 -> i13;
        14 -> i14;
        15 -> i15;
        16 -> i16;
        17 -> i17;
        18 -> i18;
        19 -> i19;
        20 -> i20;
        21 -> i21;
        22 -> i22;
        23 -> i23;
        24 -> i24;
        25 -> i25;
        26 -> i26;
        27 -> i27;
        28 -> i28;
        29 -> i29;
        30 -> i30;
        31 -> i31;
        32 -> i32;
        33 -> i33;
        34 -> i34;
        35 -> i35;
        36 -> i36;
        37 -> i37;
        38 -> i38;
        39 -> i39;
        40 -> i40;
        _ -> nomatch
    end.

float_choice(V) ->
    case V of
        +0.0 -> f0;
        1.0 -> f1;
        2.0 -> f2;
        3.0 -> f3;
        4.0 -> f4;
        5.0 -> f5;
        6.0 -> f6;
        7.0 -> f7;
        8.0 -> f8;
        9.0 -> f9;
        10.0 -> f10;
        11.0 -> f11;
        12.0 -> f12;
        13.0 -> f13;
        14.0 -> f14;
        15.0 -> f15;
        16.0 -> f16;
        17.0 -> f17;
        18.0 -> f18;
        19.0 -> f19;
        20.0 -> f20;
        21.0 -> f21;
        22.0 -> f22;
        23.0 -> f23;
        24.0 -> f24;
        25.0 -> f25;
        26.0 -> f26;
        27.0 -> f27;
        28.0 -> f28;
        29.0 -> f29;
        30.0 -> f30;
        31.0 -> f31;
        32.0 -> f32;
        33.0 -> f33;
        34.0 -> f34;
        35.0 -> f35;
        36.0 -> f36;
        37.0 -> f37;
        38.0 -> f38;
        39.0 -> f39;
        40.0 -> f40;
        _ -> nomatch
    end.

%% float_to_list/2 and float_to_binary/2 raise badarg for a term that is
%% not a float, an option that is not valid, and a text of 256 bytes or
%% more.
text_errors() ->
    [try float_to_list(id(1), []) catch error:R1 -> R1 end,
     try float_to_binary(id(1.0), [id(x)]) catch error:R2 -> R2 end,
     try float_to_binary(id(1.0), [{decimals, id(a)}]) catch error:R3 -> R3 end,
     try float_to_binary(id(1.0), [{other, 1}]) catch error:R4 -> R4 end,
     try float_to_binary(id(1.0), [short | id(x)]) catch error:R5 -> R5 end,
     try float_to_binary(id(1.0), [{decimals, -1}]) catch error:R6 -> R6 end,
     try float_to_binary(id(1.0), [{decimals, 254}]) catch error:R7 -> R7 end,
     byte_size(float_to_binary(id(1.0), [{decimals, 253}])),
     try float_to_binary(id(1.0), [{scientific, 250}]) catch error:R8 -> R8 end,
     byte_size(float_to_binary(id(1.0), [{scientific, 249}])),
     try float_to_binary(id(1.0e300), [{decimals, 2}]) catch error:R9 -> R9 end,
     try float_to_binary(id(1.0), [{decimals, 300}]) catch error:R11 -> R11 end,
     try float_to_binary(id(1.0), [{scientific, 300}]) catch error:R12 -> R12 end,
     try float_to_list(id(a)) catch error:R10 -> R10 end].

%% The text of io_lib:format/2 for floats. `~p` stops at the instruction
%% `is_any_native_record` in io_lib_pretty, which PBeam does not run yet.
format() ->
    lists:flatten(
      io_lib:format("~w ~w ~w ~w ~w ~f ~.2f ~e ~.3e ~g ~.3g ~g~n",
                    [#{1.0 => 2.5}, 1.5, 0.1, [1.0e20, -0.0, 2/3], {5.0e-324},
                     3.14159, -2.675, 1234.5, 6.02e23, 0.5, 12345.678,
                     1.0e-10])).
