-module(halt_probe).
-export([badargs/0, mask/0, status/0, options/0, no_flush/0, timeout/0, others/0, abort/0,
         slogan/0, long_slogan/0]).

%% Each badarg of halt_2 (bif.c:4316-4343, 4390-4391): the options are
%% checked before the status, so {abort, bad} and {"x", [{flush, 1}]} also
%% raise. A flush timeout is a non-negative small integer (big.c:3648-3663).
%% A slogan whose first 1023 elements are not all characters, or that ends
%% in a tail that is not [] within 1023 elements, is not a slogan
%% (bif.c:4372-4380, utils.c:2862-2942).
%% erlang:halt/1 raises the badarg again with error_info
%% (erlang.erl:3383-3388).
badargs() ->
    [try erlang:halt(S, O) catch C:R -> {C, R} end
     || {S, O} <- [{foo, []}, {-1, []}, {<<"x">>, []}, {self(), []},
                   {0, [bad]}, {0, [{flush, yes}]}, {0, [{flush_timeout, -1}]},
                   {0, [{flush_timeout, big(16#20000000, 16#40000000)}]}, {0, [{flush}]}, {0, [{other, 1}]},
                   {0, [{flush, true, x}]}, {0, [{flush, true} | x]}, {0, x},
                   {abort, bad}, {"x", [{flush, 1}]},
                   {[foo], []}, {[65 | foo], []}, {[-1], []}, {[16#D800], []},
                   {[16#110000], []}, {chars(1023, foo), []}]]
    ++ [try halt(-1) catch C:R -> {C, R} end].

%% The status code is Status masked to INT_MAX (bif.c:4346-4347).
mask() ->
    erlang:halt(16#80000005).

%% The host exit status is the low 8 bits of the status code.
status() ->
    halt(259).

%% The last option of each kind holds. term_to_Uint32 truncates the
%% largest small integer to 2^32 - 1.
options() ->
    erlang:halt(7, [{flush, false}, {flush_timeout, 0}, {flush, true},
                    {flush_timeout, big(16#20000000, 16#40000000) - 1}]).

%% Without a flush, the flush timeout has no effect.
no_flush() ->
    erlang:halt(7, [{flush_timeout, 0}, {flush, false}]).

%% A flush timeout that term_to_Uint32 truncates to 0 ends before the flush,
%% and the node exits with status 255 (erl_init.c:2771-2774).
timeout() ->
    erlang:halt(7, [{flush_timeout, big(16#10000, 16#10000)}]).

%% halt/0 stops the run with status 0 while other processes can run.
others() ->
    [spawn(fun Loop() -> Loop() end) || _ <- [1, 2]],
    erlang:yield(),
    halt().

%% A core dump (bif.c:4364-4369).
abort() ->
    halt(abort).

%% A crash dump with a slogan (bif.c:4370-4387).
slogan() ->
    halt("Received SIGUSR1").

%% A slogan longer than HALT_MSG_SIZE is cut to 1023 characters before its
%% tail is checked, so it is a crash dump (bif.c:4373-4386).
long_slogan() ->
    halt(chars(1024, foo)).

%% N characters $a and the tail Tail.
chars(0, Tail) ->
    Tail;
chars(N, Tail) ->
    chars(N - 1, [$a | Tail]).

%% A product built at run time: PBeam loads no bignum literal of a literal
%% table (see "Integer range" in docs/erts-differences.md).
big(A, B) ->
    id(A) * id(B).

id(X) ->
    X.
