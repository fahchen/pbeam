-module(rand_probe).
-export([exsss/0, default/0, global/0, invalid/0, clock/0]).

-define(MAX_64BIT_SMALL_INT, ((1 bsl 59) - 1)).
-define(MIN_64BIT_SMALL_INT, (-(1 bsl 59))).
-define(CAUSE(Expr), try Expr catch Class:Reason:Stack -> {Class, Reason, top(Stack)} end).

%% `rand:seed/2` with an explicit algorithm and a tuple seed, then 20
%% draws of `rand:uniform/1` and 20 draws of `rand:uniform/0` from the
%% state in the process dictionary.
exsss() ->
    State = rand:seed(exsss, {1, 2, 3}),
    {element(2, State), maps:get(type, element(1, State)), get(rand_seed) =:= State,
     draws(1000), floats()}.

%% `rand:seed/2` of the default algorithm with an integer seed, as
%% `global:init/1` calls it (`global.erl:865-869`), then the same draws
%% with the range of `global:init/1`.
default() ->
    State = rand:seed(default, 123456789),
    {element(2, State), maps:get(type, element(1, State)),
     draws(?MAX_64BIT_SMALL_INT - ?MIN_64BIT_SMALL_INT), floats()}.

%% The seed and draw of `global:init/1` with its time entropy
%% (`global.erl:865-869`). Only the shape of the result is fixed.
global() ->
    _ = rand:seed(default,
                  (erlang:monotonic_time(nanosecond) rem 1000000000)
                  + (erlang:system_time(nanosecond) rem 1000000000)),
    CreX = ((rand:uniform(?MAX_64BIT_SMALL_INT - ?MIN_64BIT_SMALL_INT)
             - 1) band (bnot ((1 bsl 32) -1))),
    {is_integer(CreX), CreX band 16#ffffffff, CreX >= 0, CreX < (1 bsl 60),
     maps:get(type, element(1, get(rand_seed)))}.

%% The exceptions of an unknown algorithm, a bad seed, and a bad range
%% after a valid seed.
invalid() ->
    _ = rand:seed(exsss, 1),
    [?CAUSE(rand:seed(foo, 1)), ?CAUSE(rand:seed(exsss, bad)),
     ?CAUSE(rand:seed(exsss, {1, 2})), ?CAUSE(rand:uniform(0)),
     ?CAUSE(rand:uniform(-1)), ?CAUSE(rand:uniform(1.5))].

%% `erlang:system_time/1` against the OS system time, and its exception
%% for a unit that is not valid. A positive bignum unit traps to
%% `erlang:convert_time_unit/3`, and a negative bignum unit is not valid.
clock() ->
    Erlang = erlang:system_time(nanosecond),
    Os = os:system_time(nanosecond),
    Big = 1 bsl 70,
    System = erlang:system_time(Big),
    {abs(Os - Erlang) < 1000000000, is_integer(erlang:system_time(second)),
     try erlang:system_time(foo) catch Class:Reason -> {Class, Reason} end,
     is_integer(System) andalso System > 0, erlang:monotonic_time(Big) < 0,
     erlang:convert_time_unit(1500000000, nanosecond, Big),
     try erlang:system_time(-Big) catch BigClass:BigReason -> {BigClass, BigReason} end}.

draws(Range) ->
    [rand:uniform(Range) || _ <- lists:seq(1, 20)].

floats() ->
    [rand:uniform() || _ <- lists:seq(1, 20)].

top([{M, F, A, _} | _]) ->
    {M, F, hd(A), length(A)}.
