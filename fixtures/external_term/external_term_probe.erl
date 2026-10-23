-module(external_term_probe).
-export([literals/0, options/0, decode/0, used/0, sizes/0, errors/0, decode_record/0, bits/0]).

-define(VALUES,
        [0, 255, 256, -1, 1 bsl 40, -(1 bsl 40), 1 bsl 70, -(1 bsl 70),
         1.5, 0.0, -0.0, foo, 'é', [], [1, 2, 3], [1, 2, 3, 300], [1 | 2],
         {}, {a, 1}, seq(40), #{1 => a, 2.0 => b}, <<"abc">>,
         fun erlang:abs/1]).

values() ->
    ?VALUES.

repeat(0, _) ->
    [];
repeat(N, X) ->
    [X | repeat(N - 1, X)].

seq(N) ->
    seq(1, N).

seq(N, N) ->
    [N];
seq(I, N) ->
    [I | seq(I + 1, N)].

literals() ->
    [term_to_binary(T) || T <- values()].

options() ->
    Big = repeat(40, abc),
    Exact = [term_to_binary(Big, [compressed]),
             term_to_binary(Big, [{compressed, 9}]),
             term_to_binary(Big, [{compressed, 0}]),
             term_to_binary('é', [{minor_version, 1}]),
             term_to_binary(1.5, [{minor_version, 0}]),
             term_to_binary(#{b => 1, a => 2}, [deterministic])],
    Local = term_to_binary({x, <<1, 2>>}, [local]),
    {Exact, Local, binary_to_term(Local)}.

decode() ->
    Rounds = [binary_to_term(term_to_binary(T)) =:= T || T <- values()],
    Big = repeat(200, abc),
    [Rounds,
     binary_to_term(term_to_binary(Big, [compressed])) =:= Big,
     binary_to_term(term_to_binary(foo), [safe]) =:= foo,
     binary_to_term(term_to_binary({x, <<1, 2>>}, [local])) =:= {x, <<1, 2>>},
     (binary_to_term(term_to_binary(fun erlang:abs/1)))(-5)].

used() ->
    B = term_to_binary({a, 1}),
    {T1, U1} = binary_to_term(<<B/binary, 9, 9, 9>>, [used]),
    C = term_to_binary(repeat(200, abc), [compressed]),
    {T2, U2} = binary_to_term(<<C/binary, 7, 7>>, [used]),
    [{T1, U1, byte_size(B)}, {T2, U2, byte_size(C)}].

sizes() ->
    [{erlang:external_size(T), byte_size(term_to_binary(T))} || T <- values()]
        ++ [{erlang:external_size(#{a => seq(100)}),
             byte_size(term_to_binary(#{a => seq(100)}))}].

errors() ->
    [check(F)
     || F <- [fun() -> binary_to_term(foo) end,
              fun() -> binary_to_term(<<130, 97, 1>>) end,
              fun() -> binary_to_term(<<131>>) end,
              fun() -> binary_to_term(<<131, 255>>) end,
              fun() -> binary_to_term(<<131, 97>>) end,
              fun() -> binary_to_term(<<131, 119, 3, "fo">>) end,
              fun() -> binary_to_term(binary:part(term_to_binary({a, 1}), 0, 5)) end,
              fun() -> binary_to_term(<<131, 108, 0, 0, 0, 5, 97, 1>>) end,
              fun() -> binary_to_term(<<131, 109, 0, 0, 0, 5, 1, 2>>) end,
              fun() -> binary_to_term(<<131, 70, 127, 240, 0, 0, 0, 0, 0, 1>>) end,
              fun() -> binary_to_term(<<131, 80, 0, 0, 0, 10, 1, 2, 3>>) end,
              fun() -> binary_to_term(term_to_binary(foo), [bad]) end,
              fun() -> binary_to_term(term_to_binary(foo), safe) end,
              fun() -> term_to_binary(foo, [bad]) end,
              fun() -> term_to_binary(foo, [{minor_version, 3}]) end,
              fun() -> term_to_binary(foo, [{compressed, 10}]) end,
              fun() -> term_to_binary(foo, [{compressed, -1}]) end,
              fun() -> term_to_binary(foo, [local, deterministic]) end,
              fun() -> term_to_binary(foo, [{other, 1}]) end]].

check(F) ->
    try F()
    catch Class:Reason -> {Class, Reason}
    end.

%% A term that PBeam cannot hold gives a runtime error. The bytes are a
%% well-formed `RECORD_EXT` of the record `m:r` with no fields.
decode_record() ->
    binary_to_term(<<131, 67, 0, 0, 0, 0, 1, 119, 1, "m", 119, 1, "r">>).

bits() ->
    Tail = id(<<255, 7:3>>),
    Nested = {<<1:1>>, [<<2:2>>], #{a => <<3:7>>}},
    Encoded = [term_to_binary(<<1:3>>), term_to_binary(<<1, 2:4>>),
               term_to_binary(Tail), term_to_binary(Nested),
               term_to_binary(<<0:1000, 1:3>>, [compressed]),
               term_to_binary(Nested, [deterministic]),
               term_to_binary(<<1, 2, 3, 4, 5, 6:4>>, [{minor_version, 1}])],
    Decoded = [binary_to_term(B) =:= T
               || {B, T} <- [{<<131, 77, 0, 0, 0, 1, 3, 255>>, <<7:3>>},
                             {<<131, 77, 0, 0, 0, 1, 8, 255>>, <<255>>}]]
        ++ [binary_to_term(term_to_binary(T)) =:= T
            || T <- [<<1:3>>, <<1, 2:4>>, Tail, Nested]]
        ++ [binary_to_term(<<131, 77, 0, 0, 0, 1, 3, 255>>, [safe]) =:= <<7:3>>],
    Errors = [try binary_to_term(B) catch C:R -> {C, R} end
              || B <- [<<131, 77, 0, 0, 0, 1, 0, 255>>,
                       <<131, 77, 0, 0, 0, 1, 9, 255>>,
                       <<131, 77, 0, 0, 0, 0, 3>>,
                       <<131, 77, 0, 0, 0, 2, 3, 1>>]],
    {Encoded, erlang:external_size(Tail), Decoded, Errors}.

id(X) ->
    X.
