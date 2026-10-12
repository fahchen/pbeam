%% The loops of the combined instruction pairs of
%% `docs/interpreter-dispatch-plan.md`, step 9. Each loop holds the compiled
%% shape that the loader combines: a comparison test whose fail label names
%% a `gc_bif`, or a `gc_bif` directly before the `call_only` of a tail call.
%%
%% The loops are in their own module, and `combined_main.erl` calls them.
%% The type analysis of the compiler works on one module, so a call from
%% another module cannot prove the argument type of a loop and cannot fold
%% the clause test away. A direct call with a literal would fold it, and the
%% compiled shape would then hold no pair.
%%
%% `tests/integration/combined_pairs.rs` runs `combined_main:main/1` on
%% PBeam and on the pinned ERTS and compares the two results. The result
%% covers the value, the exception class and reason, the reduction charge,
%% and the message order of each exit of each pair.
-module(combined_probe).

-export([eq/1, ne/2, lt/1, ge/1, raise_error/1, guard_fail/1, len_pair/1,
         len_call/3, bad_tail/2]).

%% Pair 1 with the comparison `is_eq_exact`: the clause test of the base
%% clause names the `gc_bif '-'` of the loop clause.
eq(0) -> 0;
eq(N) -> eq(N - 1).

%% Pair 1 with the comparison `is_ne_exact`: the first guard test names the
%% `gc_bif '+'` of the next clause.
ne(A, B) when A =/= B, is_integer(A) -> nonzero;
ne(A, _) -> A + 1.

%% Pair 1 with the comparison `is_lt`.
lt(N) when N < -5 -> small;
lt(M) -> M + 1.

%% Pair 1 with the comparison `is_ge`.
ge(N) when N >= 5 -> big;
ge(M) -> M + 1.

%% Pair 1 where the `gc_bif '+'` raises with fail label 0.
raise_error(X) when X =:= 0 -> zero;
raise_error(X) -> X + foo.

%% Pair 1 where the `gc_bif '+'` fails to the label of the next clause.
guard_fail(X) when X =:= 0 -> zero;
guard_fail(X) when X + 1 > 1 -> positive;
guard_fail(_) -> other.

%% Pair 1 with the trapping guard BIF `length/1`.
len_pair(X) when X =:= 0 -> 0;
len_pair(X) -> length(X).

%% Pair 2: the `gc_bif length` runs before the `call_only` of the tail call.
%% The list stays in the recursion, so each turn computes the length of the
%% same list and the tail call computes it last.
len_call(0, _List, Length) -> Length;
len_call(N, List, _Length) -> len_call(N - 1, List, length(List)).

%% Pair 2 where the `gc_bif '-'` raises with fail label 0, so the local
%% call does not run.
bad_tail(0, _) -> 0;
bad_tail(N, X) -> bad_tail(N - 1, X - 1).
