-module(external_term_probe).
-export([literals/0, options/0, decode/0, used/0, sizes/0, errors/0, decode_record/0, decode_remote_pid/0, bits/0, iovec/0]).

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

%% Decode a `RECORD_EXT` of the record `m:r` with no fields. The name
%% atom uses `ATOM_UTF8_EXT` (118), with a 2-byte length.
decode_record() ->
    binary_to_term(<<131, 67, 0, 0, 0, 0, 1, 119, 1, "m", 118, 0, 1, "r">>).

%% The pinned ERTS gives a pid of node 'a@b'.
decode_remote_pid() ->
    binary_to_term(<<131, 88, 119, 3, "a@b", 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1>>).

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

iovec() ->
    B = binary:copy(<<1>>, 100),
    <<_:3, S/bitstring>> = <<B/binary, 0:5>>,
    <<_:8, T/bitstring>> = <<B/binary, 0:5>>,
    Terms = [a, B, binary:copy(<<1>>, 64), binary:copy(<<1>>, 65), {B, x, B},
             [B | B], <<B/binary, 1:3>>, <<1:3>>, S, T, binary:part(B, 1, 70)],
    Options = [[], [{minor_version, 1}], [local], [{compressed, 0}], [deterministic]],
    Sizes = [[byte_size(E) || E <- erlang:term_to_iovec(X)] || X <- Terms],
    OptionSizes = [[byte_size(E) || E <- erlang:term_to_iovec({B, x}, O)] || O <- Options],
    Differ = [{I, J} || {I, X} <- number(Terms), {J, O} <- number(Options),
                        iolist_to_binary(erlang:term_to_iovec(X, O)) =/= term_to_binary(X, O)],
    Local = [try binary_to_term(iolist_to_binary(erlang:term_to_iovec(X, [local]))) =:= X
             catch C:R -> {C, R} end
             || X <- [B, <<B/binary, 1:3>>, <<B/binary, 1:3, 7>>, binary:copy(<<1>>, 98)]],
    Compressed = [erlang:term_to_iovec(X, [compressed]) =:= term_to_binary(X, [compressed])
                  || X <- [a, B, binary:copy(<<0>>, 1000)]],
    External = [erlang:external_size(X, O)
                || {X, O} <- [{binary:copy(<<0>>, 1000), [compressed]}, {x, [{minor_version, 1}]},
                              {1.0, [{minor_version, 0}]}, {a, [local]}, {a, [deterministic]},
                              {B, [{compressed, 9}]}, {B, []}]],
    Errors = [try F() catch C:R -> {C, R} end
              || F <- [fun() -> erlang:term_to_iovec(x, bad) end,
                       fun() -> erlang:term_to_iovec(x, [local, deterministic]) end,
                       fun() -> erlang:external_size(x, [{compressed, 10}]) end,
                       fun() -> erlang:external_size(x, [foo | bar]) end]],
    Str = repeat(70, $a),
    Referenced = [[binary:referenced_byte_size(E) || E <- erlang:term_to_iovec({Str, B, Str})]
                  | [binary:referenced_byte_size(hd(tl(erlang:term_to_iovec(X)))) || X <- [B, T]]]
        ++ [[{byte_size(E), binary:referenced_byte_size(E)} || E <- erlang:term_to_iovec(X, O)]
            || {X, O} <- [{<<0:512, 1:1>>, []}, {{a, <<0:4>>}, []}, {{B, a, B}, []},
                          {{a, B}, [local]}]],
    {Referenced, Sizes, OptionSizes, Differ, Local, Compressed, External, Errors,
     erlang:term_to_iovec(a), erlang:apply(erlang, term_to_iovec, [a, []])}.

number(List) ->
    number(1, List).

number(_, []) ->
    [];
number(I, [X | Rest]) ->
    [{I, X} | number(I + 1, Rest)].
