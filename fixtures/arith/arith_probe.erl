-module(arith_probe).
-export([arith/0, guards/0, externals/0, plus_overflow/0, minus_overflow/0]).

%% `gc_bif` `erlang:'+'/2` and `erlang:'-'/2` with fail label 0, as
%% io_lib_format:build_bin/2 (io_lib_format.erl:133),
%% io_lib_format:count_small/2 (io_lib_format.erl:325),
%% user_drv:gr_add_cur/3 (user_drv.erl:1108), and
%% io_lib_format:build_limited_bin/5 (io_lib_format.erl:422) run them.
%% 2^59 and the values up to 2^63 - 1 are ERTS bignums. A term that is not
%% a number raises badarith.
arith() ->
    [id(1) + id(2),
     id(5) - id(7),
     id(-3) + id(-4),
     id(576460752303423487) + id(1),
     id(-576460752303423488) - id(1),
     id(576460752303423488) - id(1),
     id(9223372036854775806) + id(1),
     id(-9223372036854775807) - id(1),
     try id(a) + id(1) catch error:R1 -> R1 end,
     try id(1) - id([]) catch error:R2 -> R2 end,
     try id(<<1>>) + id({}) catch error:R3 -> R3 end,
     try id(self()) - id(1) catch error:R4 -> R4 end].

%% `gc_bif` `erlang:'+'/2` and `erlang:'-'/2` in a guard: badarith jumps to
%% the fail label, as in logger_config:allow/2 (logger_config.erl:70).
guards() ->
    [up(id(1)), up(id(-5)), up(id(a)), up(id([])),
     down(id(3)), down(id(0)), down(id(b)), down(id({}))].

%% erlang:'+'/2 and erlang:'-'/2 as external calls: the apply instruction
%% and a call of an external fun.
externals() ->
    Minus = id(fun erlang:'-'/2),
    [apply(id(erlang), id('+'), [id(1), id(2)]),
     Minus(id(5), id(7)),
     try apply(id(erlang), id('-'), [id(a), id(1)]) catch error:R -> R end].

%% Results outside the 64-bit signed range: ERTS makes a bignum.
plus_overflow() -> id(9223372036854775807) + id(1).
minus_overflow() -> id(-9223372036854775808) - id(1).

up(X) when X + 1 >= 1 -> {up, X + 1};
up(_) -> other.

down(X) when X - 1 >= 0 -> {down, X - 1};
down(_) -> other.

id(X) -> X.
