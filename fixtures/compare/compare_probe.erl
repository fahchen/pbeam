-module(compare_probe).
-export([exact/0, equal/0, guards/0, id/1]).

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

%% The tests `is_lt`, `is_integer`, and `is_boolean`. `next/1` is the
%% `gc_bif` guard of `erlang:'+'/2` with a fail label, then `is_lt` against
%% the integer 0, as io_lib_format:build_limited_bin/5
%% (io_lib_format.erl:416-429) does. `kind/1` is the `is_integer` test of
%% io_lib_format:field_value/2 (io_lib_format.erl:271) and the
%% `is_boolean` test of standard_error:check_valid_opts/1
%% (standard_error.erl:236). `width/1` is io_lib_format:field_width/3
%% (io_lib_format.erl:259), which compares the atom `none` with 0.
%% `less/2` is `is_lt` on two registers, as io_lib_format:sub/2
%% (io_lib_format.erl:1108) and lists:usort/1 (lists.erl:2076, 2093) do.
guards() ->
    [[next(id(1)), next(id(-1)), next(id(a)), next(id([]))],
     [kind(id(0)), kind(id(-5)), kind(id(576460752303423488)), kind(id(true)), kind(id(false)),
      kind(id(nil)), kind(id([])), kind(id(<<>>)), kind(id({1})), kind(self()), kind(make_ref())],
     [width(id(none)), width(id(-2)), width(id(0)), width(id(3))],
     [less(id(1), id(2)), less(id(2), id(1)), less(id(1), id(1)), less(id(-1), id(0)),
      less(id(1), id(a)), less(id(a), id(1)), less(id(a), id(b)), less(id(b), id(a)),
      less(id({1}), id({2})), less(id({1, 2}), id({3})), less(id([]), id([1])),
      less(id([1]), id([])), less(id(<<"a">>), id(<<"b">>)), less(self(), id({})),
      less(id(#{}), id([])), less(id(576460752303423488), id(576460752303423487))]].

next(X) when X + 1 > 0 -> {pos, X + 1};
next(X) when is_integer(X) -> {nonpos, X + 1};
next(_) -> other.

kind(X) when is_integer(X) -> integer;
kind(X) when is_boolean(X) -> boolean;
kind(_) -> other.

width(F) when F < 0 -> left;
width(F) when F >= 0 -> right.

less(A, B) when A < B -> lt;
less(_, _) -> nlt.

id(X) -> X.

%% The `is_eq` test, as the guard `CtrlSync==0` of
%% logger_h_common:handle_load/2 (logger_h_common.erl:250).
equal() ->
    Ref = make_ref(),
    Pairs = [{1, 1}, {1, 2}, {1, a}, {a, a}, {a, b}, {{1, a}, {id(1), a}}, {{1, a}, {1, b}},
             {[1, 2], [1 | id([2])]}, {[], [1]}, {<<"x">>, <<"x">>}, {<<"x">>, <<"y">>},
             {#{a => 1}, #{a => id(1)}}, {#{a => 1}, #{a => 2}}, {self(), self()},
             {Ref, Ref}, {Ref, make_ref()}, {fun id/1, fun id/1}, {fun id/1, fun ?MODULE:id/1},
             {576460752303423488, id(576460752303423488)}, {[a | b], [a | c]}],
    [eq(id(A), id(B)) || {A, B} <- Pairs].

eq(A, B) when A == B -> id(true);
eq(_, _) -> id(false).
