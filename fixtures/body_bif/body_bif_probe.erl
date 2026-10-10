-module(body_bif_probe).
-export([max/0, less/0, equal/0, neg/0, neg_overflow/0, bit_and/0, executable/1, binary/0,
         externals/0, costs/0]).

%% The reduction total of the calling process. element/2 is a guard BIF,
%% so it costs no reduction.
-define(REDS, element(2, process_info(self(), reductions))).

%% Pairs of terms of the same type and of different types. 576460752303423488
%% (2^59) and 9223372036854775807 (2^63 - 1) are ERTS bignums, and
%% 576460752303423487 (2^59 - 1) and -576460752303423488 (-(2^59)) are small
%% integers. The pairs of large integers are built at run time: PBeam loads
%% no bignum in the literal table.
pairs() ->
    [{1, 2}, {2, 1}, {-3, -3}, {id(576460752303423488), id(576460752303423487)},
     {id(-9223372036854775808), 1}, {id(9223372036854775807), id(-576460752303423488)},
     {1, a}, {a, 1},
     {a, b}, {b, a}, {[], [1]}, {[1, 2], [1]}, {<<"a">>, {}}, {{1, 2}, {3}}, {#{}, []},
     {self(), {}}, {{1, a}, {1, a}}, {<<"ab">>, <<"ab">>}].

%% `bif` `erlang:max/2` with fail label 0, as Terra.View.measure/2,
%% Terra.View.align_x/3, and Terra.View.do_place/3 run it.
max() -> [erlang:max(id(A), id(B)) || {A, B} <- pairs()].

%% `bif` `erlang:'<'/2` with fail label 0, as Terra.Grid.inside?/3 runs
%% it.
less() -> [id(A) < id(B) || {A, B} <- pairs()].

%% `bif` `erlang:'=='/2` with fail label 0, as Terra.Grid.put/5 runs it.
equal() -> [id(A) == id(B) || {A, B} <- pairs()].

%% `gc_bif` `erlang:'-'/1` with fail label 0, as
%% io_lib_format:field_width/2 (io_lib_format.erl:254) runs it. The
%% negation of 2^59 is the small integer -(2^59), and the negation of
%% -(2^59) is the bignum 2^59. A term that is not a number raises badarith.
neg() ->
    [-id(0), -id(5), -id(-5), -id(576460752303423488), -id(-576460752303423488),
     -id(9223372036854775807),
     try -id(a) catch error:R1 -> R1 end,
     try -id([]) catch error:R2 -> R2 end,
     try -id(<<1>>) catch error:R3 -> R3 end].

%% The negation of -(2^63): ERTS makes the bignum 2^63.
neg_overflow() -> -id(-9223372036854775808).

%% `gc_bif` `erlang:'band'/2` with fail label 0. The bitwise and of two
%% negative integers is negative. A term that is not an integer raises
%% badarith.
bit_and() ->
    [id(12) band id(10), id(-1) band id(255), id(-8) band id(-3),
     id(9223372036854775807) band id(576460752303423488),
     id(-9223372036854775808) band id(-1), id(-576460752303423489) band id(-1),
     id(576460752303423487) band id(-576460752303423488),
     try id(a) band id(1) catch error:R1 -> R1 end,
     try id(1) band id(a) catch error:R2 -> R2 end,
     try id([]) band id({}) catch error:R3 -> R3 end].

%% `gc_bif` `erlang:'band'/2` in a guard, as os:verify_executable/3
%% (os.erl:455-456) tests the mode of a file. badarith jumps to the fail
%% label.
executable(Mode) when Mode band 8#111 =/= 0 -> yes;
executable(_) -> no.

%% `bif` `erlang:is_binary/1` with fail label 0, as
%% unicode:no_conversion_needed/3 (unicode.erl:359-360) runs it.
binary() ->
    [is_binary(id(X)) || X <- [<<>>, <<1, 2>>, "ab", a, 1, {}, #{}, self(), fun id/1]].

%% The BIFs as external calls: the apply instruction and a call of an
%% external fun.
externals() ->
    Less = id(fun erlang:'<'/2),
    Band = id(fun erlang:'band'/2),
    [apply(id(erlang), id(max), [id(1), id(a)]),
     Less(id(a), id(1)),
     apply(id(erlang), id('=='), [id({1}), id({1})]),
     apply(id(erlang), id('-'), [id(7)]),
     Band(id(6), id(3)),
     apply(id(erlang), id(is_binary), [id(<<"x">>)]),
     try apply(id(erlang), id('-'), [id(a)]) catch error:R1 -> R1 end,
     try Band(id(1), id(a)) catch error:R2 -> R2 end].

%% The reductions of `band` and unary `-`. A bignum operand costs 1
%% reduction (bump_reds_linear in erl_arith.c:56-72), and small integers
%% cost none.
costs() ->
    [band_cost(id(12), id(10)), band_cost(id(576460752303423488), id(-1)),
     band_cost(id(-1), id(9223372036854775807)), neg_cost(id(5)),
     neg_cost(id(-576460752303423488)), neg_cost(id(576460752303423488))].

band_cost(A, B) ->
    R0 = ?REDS,
    C = A band B,
    R1 = ?REDS,
    {C, R1 - R0}.

neg_cost(A) ->
    R0 = ?REDS,
    C = -A,
    R1 = ?REDS,
    {C, R1 - R0}.

id(X) -> X.
