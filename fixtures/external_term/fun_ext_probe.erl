-module(fun_ext_probe).
-export([round_trip/0, unloaded/0, call_unloaded/0, errors/0, other_md5/0, no_lambda_handler/0, deleted/0, wrong_arity/0]).

make(Tag) ->
    fun(X, Y) -> {Tag, X, Y} end.

%% A local fun with free variables and one with none: the bytes, the
%% external size, the equality and the fun information of the decoded fun,
%% and a call of each decoded fun.
round_trip() ->
    F = make({tag, <<1, 2>>}),
    G = fun() -> ok end,
    [begin
         B = term_to_binary(Fun),
         D = binary_to_term(B),
         {B, erlang:external_size(Fun), D =:= Fun, erlang:fun_info(D, index),
          erlang:fun_info(D, new_uniq), erlang:fun_info(D, arity)}
     end || Fun <- [F, G]]
        ++ [(binary_to_term(term_to_binary(F)))(1, 2),
            (binary_to_term(term_to_binary(G)))()].

%% A fun of a module that is not loaded, with a size field of 0 that the
%% decoder skips.
unloaded_fun() ->
    binary_to_term(<<131, 112, 0, 0, 0, 0, 1, 7:128, 0, 0, 0, 3, 0, 0, 0, 1,
                     119, 7, "no_such", 97, 2, 98, 255, 255, 255, 255,
                     88, 119, 13, "nonode@nohost", 0:96, 97, 5>>).

%% The fun information and the bytes of a new encode of the fun.
unloaded() ->
    F = unloaded_fun(),
    {erlang:fun_info(F, module) =:= {module, no_such},
     [erlang:fun_info(F, K)
      || K <- [index, new_index, new_uniq, uniq, arity, env, type]],
     term_to_binary(F)}.

%% A call of the fun, which goes to `error_handler:undefined_lambda/3`.
call_unloaded() ->
    try (unloaded_fun())(1) catch C:R:S -> {C, R, hd(S)} end.

%% An error handler without `undefined_lambda/3` gives undef
%% (`beam_common.c:1713-1717`).
no_lambda_handler() ->
    F = unloaded_fun(),
    Old = process_flag(error_handler, ?MODULE),
    Result = try F(1) catch C:R -> {C, R} end,
    process_flag(error_handler, Old),
    Result.

%% A module that is not an atom, an old index or an old unique value that is
%% not a small integer, a creator that is not a pid, and a truncated fun give
%% badarg. An old index of 2^51 + 1 as a big is small and wraps to 1.
errors() ->
    Head = <<131, 112, 0, 0, 0, 0, 1, 7:128, 0, 0, 0, 3, 0, 0, 0, 0>>,
    Pid = <<88, 119, 13, "nonode@nohost", 0:96>>,
    [try binary_to_term(B) of
         F -> erlang:fun_info(F, index)
     catch
         C:R -> {C, R}
     end
     || B <- [<<Head/binary, 107, 0, 1, 97, 97, 0, 97, 0, Pid/binary>>,
              <<Head/binary, 119, 1, "m", 100, 0, 1, "a", 97, 0, Pid/binary>>,
              <<Head/binary, 119, 1, "m", 97, 0, 110, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
                Pid/binary>>,
              <<Head/binary, 119, 1, "m", 97, 0, 97, 0, 97, 0>>,
              <<Head/binary, 119, 1, "m", 97, 0, 97, 0>>,
              <<Head/binary, 119, 1, "m", 110, 8, 0, 1, 0, 0, 0, 0, 0, 8, 0, 97, 0,
                Pid/binary>>]].

%% A decoded fun with another module MD5 gets another fun entry
%% (`erl_fun.c:79-85`), so it is not exactly equal to the fun
%% (`utils.c:1257-1260`). The compare ignores the MD5 (`utils.c:2157-2179`),
%% and a call raises badfun: the module is loaded, and the stub entry has no
%% code (`beam_common.c:1697-1706`).
other_md5() ->
    F = fun() -> ok end,
    <<Head:22/binary, Last, Rest/binary>> = term_to_binary(F),
    D = binary_to_term(<<Head/binary, (Last bxor 1), Rest/binary>>),
    {D =:= F, D == F, try D() catch C:R -> {C, R =:= {badfun, D}} end}.

%% After `erlang:delete_module/1`, the fun entry keeps the old code, and a
%% call of the fun runs it.
deleted() ->
    F = make(t),
    true = erlang:delete_module(?MODULE),
    F(1, 2).

%% A decoded fun with the MD5 and index of a loaded entry but another arity
%% gets another fun entry (`erl_fun.c:79-85`), and a call raises badfun
%% (`beam_common.c:1697-1706`).
wrong_arity() ->
    <<Head:6/binary, 2, Rest/binary>> = term_to_binary(make(t)),
    D = binary_to_term(<<Head/binary, 1, Rest/binary>>),
    try D(1) catch C:R -> {C, R =:= {badfun, D}} end.
