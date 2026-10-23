-module(undef_frames).
-export([all/0, run/2, id/1, burn/1, k_call/0, k_last/0, k_only/0, k_apply/0, k_apply_last/0,
         k_fun/0, k_fun_last/0, k_bif_apply/0, k_bif_apply2/0, k_deep/0,
         k_deep_last/0, k_ext/0, k_ext_last/0, k_ext_only/0]).

%% The full stack trace of `error:undef` for each call kind, from a new
%% process after `burn(N)` reductions. The default error handler raises
%% `undef` through `error_handler:crash/1` (`error_handler.erl:165-178`).
%% The values of N near 4000 put the preemption of the process inside the
%% error handler code. `k_call`, `k_last`, and `k_only` compile to `apply` and
%% `apply_last`; `k_ext*` compile to the `call_ext` forms.

id(X) -> X.

burn(0) -> ok;
burn(N) -> burn(N - 1).

all() ->
    %% A run of the library API has no init process, so start the responder
    %% of `fixtures/undef/undef_init` that answers `init:ensure_loaded/1`.
    case erlang:function_exported(undef_init, start, 0) of
        true -> undef_init:start();
        false -> ok
    end,
    [{K, N, run(K, N)} || K <- [k_call, k_last, k_only, k_apply, k_apply_last, k_fun, k_fun_last,
                               k_bif_apply, k_bif_apply2, k_deep, k_deep_last,
                               k_ext, k_ext_last, k_ext_only],
                          N <- lists:seq(0, 9) ++ lists:seq(3940, 4000)].

run(K, N) ->
    Parent = self(),
    Pid = spawn(fun() ->
                        burn(N),
                        R = try ?MODULE:K() catch C:E:S -> {C, E, S} end,
                        Parent ! {self(), R}
                end),
    receive {Pid, R} -> R end.

m() -> binary_to_atom(id(<<"undef_frames_target">>)).

k_call() -> R = (m()):missing(a), id(R).
k_last() -> X = id(1), (m()):missing(X).
k_only() -> (m()):missing(a).
k_apply() -> R = apply(m(), id(missing), [a]), id(R).
k_apply_last() -> X = id(a), apply(m(), id(missing), [X]).
k_fun() -> F = fun undef_frames_target:missing/1, R = (id(F))(a), id(R).
k_fun_last() -> F = fun undef_frames_target:missing/1, (id(F))(a).
k_bif_apply() -> R = erlang:apply(id(m()), id(missing), id([a])), id(R).
k_bif_apply2() -> R = erlang:apply(id(fun undef_frames_target:missing/1), id([a])), id(R).
k_deep() -> R = k_call(), id(R).
k_deep_last() -> _ = id(a), k_call().
k_ext() -> R = undef_frames_target:missing(a), id(R).
k_ext_last() -> X = id(a), undef_frames_target:missing(X).
k_ext_only() -> undef_frames_target:missing(a).
