-module(compare_probe).
-export([exact/0, id/1]).

%% `bif` `erlang:'=:='/2` and `erlang:'=/='/2` with fail label 0, as
%% user_drv:init/1 (user_drv.erl:184-186), user_drv:init_shell/2
%% (user_drv.erl:364), group:init/1 (group.erl:143), and prim_tty:init/1
%% (prim_tty.erl:240) run them. Each pair compares two terms of the same
%% or of a different type. A local fun and an external fun to the same
%% function are not exactly equal.
exact() ->
    Ref = make_ref(),
    Pairs = [{1, 1}, {1, 2}, {1, a}, {a, a}, {a, b}, {{1, a}, {id(1), a}}, {{1, a}, {1, b}},
             {[1, 2], [1 | id([2])]}, {[], [1]}, {<<"x">>, <<"x">>}, {<<"x">>, <<"y">>},
             {#{a => 1}, #{a => id(1)}}, {#{a => 1}, #{a => 2}}, {self(), self()},
             {Ref, Ref}, {Ref, make_ref()}, {fun id/1, fun id/1}, {fun id/1, fun ?MODULE:id/1}],
    [[id(A) =:= id(B) || {A, B} <- Pairs],
     [id(A) =/= id(B) || {A, B} <- Pairs]].

id(X) -> X.
