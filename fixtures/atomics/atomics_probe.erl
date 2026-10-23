-module(atomics_probe).
-export([main/0, allocation_failure/0, write_concurrency/0,
         write_concurrency_allocation_failure/0, exchange/0,
         race_owner/0, race_peer/0]).

%% The `counters` calls of the Elixir Logger start of the plug_hello
%% release: `counters:new(1, [atomics])` in
%% `Elixir.Logger.Backends.Internal:init/1`, and `counters:get/2` and
%% `counters:add/3` in `Elixir.Logger.Backends.Handler:threshold/1`. The
%% default mode of the `counters` module is the `atomics` module, whose
%% BIFs are `erts_internal:atomics_new/2`, `atomics:get/2`,
%% `atomics:add/3`, `atomics:put/3`, and `atomics:info/1`
%% (`erl_bif_atomics.c`). `main/0` gives one pair for each case: the result
%% of the call, or the class, the reason, and the first frame of the
%% exception of the call.
main() ->
    [{new_atomics, call(fun() -> counters:new(1, [atomics]) end)},
     {new_default, call(fun() -> counters:new(2, []) end)},
     {counter, call(fun counter/0)},
     {info_signed, call(fun() -> counters:info(counters:new(2, [atomics])) end)},
     {info_unsigned, call(fun() -> atomics:info(atomics:new(2, [{signed, false}])) end)},
     {get_signed, call(fun() -> atomics:get(signed(-1), 1) end)},
     {get_unsigned, call(fun() -> atomics:get(unsigned((1 bsl 64) - 1), 1) end)},
     {add_signed, call(fun() -> atomics:get(add(signed(0), 1, 2), 1) end)},
     {add_wrap, call(fun() -> atomics:get(add(signed((1 bsl 63) - 1), 1, 1), 1) end)},
     {add_unsigned_wrap, call(fun() -> atomics:get(add(unsigned((1 bsl 64) - 1), 1, 1), 1) end)},
     {put_signed, call(fun() -> atomics:put(signed(0), 1, -(1 bsl 63)) end)},
     {put_unsigned, call(fun() -> atomics:get(put(unsigned(0), 1, 1 bsl 63), 1) end)},
     {get_not_a_reference, call(fun() -> atomics:get(5, 1) end)},
     {get_unknown_reference, call(fun() -> atomics:get(make_ref(), 1) end)},
     {get_counter_not_a_reference, call(fun() -> counters:get(5, 1) end)},
     {index_zero, call(fun() -> counters:get(counters:new(1, []), 0) end)},
     {index_above_size, call(fun() -> counters:get(counters:new(1, []), 2) end)},
     {index_not_an_integer, call(fun() -> counters:get(counters:new(1, []), 1.5) end)},
     {put_above_signed_range, call(fun() -> atomics:put(signed(0), 1, 1 bsl 63) end)},
     {put_below_unsigned_range, call(fun() -> atomics:put(unsigned(0), 1, -1) end)},
     {put_above_machine_word, call(fun() -> atomics:put(signed(0), 1, 1 bsl 64) end)},
     {add_above_both_ranges, call(fun() -> atomics:add(signed(0), 1, 1 bsl 70) end)},
     {add_not_an_integer, call(fun() -> atomics:add(signed(0), 1, 1.0) end)},
     {size_zero, call(fun() -> counters:new(0, []) end)},
     {size_negative, call(fun() -> counters:new(-1, []) end)},
     {size_not_an_integer, call(fun() -> counters:new(1.0, []) end)},
     {size_negative_bignum, call(fun() -> counters:new(-(1 bsl 62), []) end)},
     {size_negative_above_i64, call(fun() -> counters:new(-(1 bsl 70), []) end)},
     {size_above_machine_word, call(fun() -> counters:new(1 bsl 64, []) end)},
     {size_above_limit, call(fun() -> counters:new(2305843009213693952, []) end)},
     {options_not_a_mode, call(fun() -> counters:new(1, [x]) end)},
     {options_not_a_list, call(fun() -> counters:new(1, atomics) end)},
     {options_negative, call(fun() -> erts_internal:atomics_new(1, -1) end)},
     {options_not_an_integer, call(fun() -> erts_internal:atomics_new(1, 1.0) end)},
     {options_above_machine_word, call(fun() -> erts_internal:atomics_new(1, 1 bsl 64) end)}].

%% The `counters` calls of the release one by one, and the value of the
%% counter after each write.
counter() ->
    Ref = counters:new(1, [atomics]),
    [counters:get(Ref, 1),
     counters:add(Ref, 1, 5),
     counters:get(Ref, 1),
     counters:sub(Ref, 1, 2),
     counters:get(Ref, 1),
     counters:put(Ref, 1, -1),
     counters:get(Ref, 1),
     counters:put(Ref, 1, (1 bsl 63) - 1),
     counters:add(Ref, 1, 1),
     counters:get(Ref, 1)].

%% An array of 2^60 atomics, which is 8 EiB and no machine holds. ERTS ends
%% the emulator when its allocation fails (`erts_alloc_n_enomem`,
%% `erl_alloc.c:2105-2109`), so only a PBeam run calls this case.
allocation_failure() -> call(fun() -> counters:new(1 bsl 60, []) end).

%% The `write_concurrency` mode of `counters:new/2`, whose BIFs are
%% `erts_internal:counters_new/1`, `counters_get/2`, `counters_add/3`,
%% `counters_put/3`, and `counters_info/1` (`erl_bif_counters.c`). The
%% `memory` of `counters:info/1` depends on the count of schedulers, so the
%% pinned ERTS runs it with `+S 1`.
write_concurrency() ->
    Ref = counters:new(1, [write_concurrency]),
    Atomics = counters:new(1, [atomics]),
    {write_concurrency, Raw} = Ref,
    {atomics, RawAtomics} = Atomics,
    [counters:info(Ref),
     counters:info(counters:new(9, [write_concurrency])),
     counters:get(Ref, 1),
     counters:add(Ref, 1, 5),
     counters:get(Ref, 1),
     counters:sub(Ref, 1, 2),
     counters:get(Ref, 1),
     counters:put(Ref, 1, -1),
     counters:get(Ref, 1),
     counters:put(Ref, 1, (1 bsl 63) - 1),
     counters:add(Ref, 1, 1),
     counters:get(Ref, 1),
     counters:add(Ref, 1, (1 bsl 64) - 1),
     counters:get(Ref, 1),
     reason(fun() -> counters:new(0, [write_concurrency]) end),
     reason(fun() -> counters:new(-1, [write_concurrency]) end),
     reason(fun() -> counters:new(1.0, [write_concurrency]) end),
     reason(fun() -> counters:new(1 bsl 64, [write_concurrency]) end),
     reason(fun() -> counters:new(1 bsl 59, [write_concurrency]) end),
     reason(fun() -> counters:new((1 bsl 59) + 1, [write_concurrency]) end),
     reason(fun() -> counters:get(Ref, 0) end),
     reason(fun() -> counters:get(Ref, 2) end),
     reason(fun() -> counters:add(Ref, 2, 1) end),
     reason(fun() -> counters:put(Ref, 0, 1) end),
     reason(fun() -> counters:put(Ref, 1, 1 bsl 63) end),
     reason(fun() -> counters:put(Ref, 1, 1.0) end),
     reason(fun() -> counters:add(Ref, 1, 1 bsl 64) end),
     reason(fun() -> counters:add(Ref, 1, 1.0) end),
     reason(fun() -> erts_internal:counters_get(RawAtomics, 1) end),
     reason(fun() -> erts_internal:counters_info(RawAtomics) end),
     reason(fun() -> erts_internal:counters_info(5) end),
     reason(fun() -> atomics:get(Raw, 1) end)].

%% A `write_concurrency` array of 2^59 - 1 counters, the largest size that
%% passes the size check of one scheduler (`erl_bif_counters.c:92-93`),
%% which is 4 EiB in PBeam. ERTS ends the emulator when its allocation
%% fails ("Atomic arrays" in `docs/erts-differences.md`), so only a PBeam
%% run calls this case.
write_concurrency_allocation_failure() ->
    reason(fun() -> counters:new((1 bsl 59) - 1, [write_concurrency]) end).

%% `atomics:add_get/3`, `atomics:exchange/3`, and
%% `atomics:compare_exchange/4` (`erl_bif_atomics.c:187-227`). Give `ok`
%% when each case gives the expected term, or the list of the cases that
%% do not.
exchange() ->
    MaxS = (1 bsl 63) - 1,
    MinS = -(1 bsl 63),
    MaxU = (1 bsl 64) - 1,
    Badarg = fun(F, A) ->
                     {error, badarg,
                      {atomics, F, A, [{error_info, #{module => erl_erts_errors}}]}}
             end,
    Cases =
        [{add_get, call(fun() -> atomics:add_get(signed(2), 1, 5) end), 7},
         {add_get_signed_wrap, call(fun() -> atomics:add_get(signed(MaxS), 1, 1) end), MinS},
         {add_get_unsigned_wrap, call(fun() -> atomics:add_get(unsigned(MaxU), 1, 1) end), 0},
         {add_get_unsigned_negative, call(fun() -> atomics:add_get(unsigned(0), 1, -1) end), MaxU},
         {add_get_signed_unsigned_incr, call(fun() -> atomics:add_get(signed(0), 1, MaxU) end), -1},
         {add_get_unsigned_bignum, call(fun() -> atomics:add_get(unsigned(0), 1, 1 bsl 63) end), 1 bsl 63},
         {add_get_stores, call(fun() -> atomics:get(add_get(signed(1), 1, -3), 1) end), -2},
         {exchange_signed, call(fun() -> atomics:exchange(signed(-3), 1, 7) end), -3},
         {exchange_unsigned, call(fun() -> atomics:exchange(unsigned(MaxU), 1, 1) end), MaxU},
         {exchange_stores, call(fun() -> atomics:get(exchange(unsigned(0), 1, MaxU), 1) end), MaxU},
         {cas_hit, call(fun() -> atomics:compare_exchange(signed(1), 1, 1, 2) end), ok},
         {cas_hit_stores, call(fun() -> atomics:get(cas(signed(1), 1, 1, MinS), 1) end), MinS},
         {cas_miss, call(fun() -> atomics:compare_exchange(signed(MinS), 1, 0, 2) end), MinS},
         {cas_miss_keeps, call(fun() -> atomics:get(cas(signed(5), 1, 0, 2), 1) end), 5},
         {cas_miss_unsigned, call(fun() -> atomics:compare_exchange(unsigned(MaxU), 1, 0, 1) end), MaxU},
         {cas_hit_unsigned, call(fun() -> atomics:compare_exchange(unsigned(MaxU), 1, MaxU, 0) end), ok},
         {add_get_not_a_reference, call(fun() -> atomics:add_get(5, 1, 1) end),
          Badarg(add_get, [5, 1, 1])},
         {add_get_index_zero, call(fun() -> atomics:add_get(signed(0), 0, 1) end),
          Badarg(add_get, [reference, 0, 1])},
         {add_get_index_above_size, call(fun() -> atomics:add_get(signed(0), 2, 1) end),
          Badarg(add_get, [reference, 2, 1])},
         {add_get_above_both_ranges, call(fun() -> atomics:add_get(signed(0), 1, 1 bsl 64) end),
          Badarg(add_get, [reference, 1, 1 bsl 64])},
         {add_get_not_an_integer, call(fun() -> atomics:add_get(signed(0), 1, 1.0) end),
          Badarg(add_get, [reference, 1, 1.0])},
         {exchange_not_a_reference, call(fun() -> atomics:exchange(make_ref(), 1, 1) end),
          Badarg(exchange, [reference, 1, 1])},
         {exchange_index_not_an_integer, call(fun() -> atomics:exchange(signed(0), x, 1) end),
          Badarg(exchange, [reference, x, 1])},
         {exchange_above_signed_range, call(fun() -> atomics:exchange(signed(0), 1, 1 bsl 63) end),
          Badarg(exchange, [reference, 1, 1 bsl 63])},
         {exchange_below_unsigned_range, call(fun() -> atomics:exchange(unsigned(0), 1, -1) end),
          Badarg(exchange, [reference, 1, -1])},
         {cas_not_a_reference, call(fun() -> atomics:compare_exchange(5, 1, 0, 0) end),
          Badarg(compare_exchange, [5, 1, 0, 0])},
         {cas_index_above_size, call(fun() -> atomics:compare_exchange(signed(0), 2, 0, 0) end),
          Badarg(compare_exchange, [reference, 2, 0, 0])},
         {cas_bad_expected, call(fun() -> atomics:compare_exchange(unsigned(0), 1, -1, 0) end),
          Badarg(compare_exchange, [reference, 1, -1, 0])},
         {cas_bad_desired, call(fun() -> atomics:compare_exchange(signed(0), 1, 0, MaxU) end),
          Badarg(compare_exchange, [reference, 1, 0, MaxU])},
         {cas_bad_desired_not_an_integer, call(fun() -> atomics:compare_exchange(signed(0), 1, 0, 1.0) end),
          Badarg(compare_exchange, [reference, 1, 0, 1.0])}],
    case [{Label, Got} || {Label, Got, Want} <- Cases, Got =/= Want] of
        [] -> ok;
        Failed -> Failed
    end.

%% The rounds of each process of the race.
-define(ROUNDS, 20000).

%% The first root of a race of two schedulers on one array. It makes the
%% array and publishes it through `persistent_term`, and then runs the
%% race.
race_owner() ->
    Ref = atomics:new(2, []),
    persistent_term:put(atomics_probe_race, Ref),
    race(Ref).

%% The second root of the race. It waits for the array of the owner.
race_peer() ->
    race(wait_for_array()).

wait_for_array() ->
    case persistent_term:get(atomics_probe_race, undefined) of
        undefined -> erlang:yield(), wait_for_array();
        Ref -> Ref
    end.

%% Increment atomic 1 with `compare_exchange/4` retries and atomic 2 with
%% `add_get/3`, then wait until the other root is done. Each atomic holds
%% the count of the increments of the two roots when both are done.
race(Ref) ->
    cas_rounds(Ref, ?ROUNDS),
    add_get_rounds(Ref, ?ROUNDS),
    wait_until(Ref, 2 * ?ROUNDS),
    {atomics:get(Ref, 1), atomics:exchange(Ref, 2, 2 * ?ROUNDS)}.

cas_rounds(_Ref, 0) -> ok;
cas_rounds(Ref, N) ->
    cas_increment(Ref, atomics:get(Ref, 1)),
    cas_rounds(Ref, N - 1).

cas_increment(Ref, Old) ->
    case atomics:compare_exchange(Ref, 1, Old, Old + 1) of
        ok -> ok;
        Current -> cas_increment(Ref, Current)
    end.

add_get_rounds(_Ref, 0) -> ok;
add_get_rounds(Ref, N) ->
    New = atomics:add_get(Ref, 2, 1),
    true = New >= 1 andalso New =< 2 * ?ROUNDS,
    add_get_rounds(Ref, N - 1).

wait_until(Ref, Total) ->
    case atomics:get(Ref, 1) =:= Total andalso atomics:get(Ref, 2) =:= Total of
        true -> ok;
        false -> erlang:yield(), wait_until(Ref, Total)
    end.

%% A signed array of one atomic with the value `Value`.
signed(Value) ->
    Ref = atomics:new(1, [{signed, true}]),
    ok = atomics:put(Ref, 1, Value),
    Ref.

%% An unsigned array of one atomic with the bits of `Value`.
unsigned(Value) ->
    Ref = atomics:new(1, [{signed, false}]),
    ok = atomics:put(Ref, 1, Value),
    Ref.

%% Add `Increment` to the atomic `Index` of `Ref`.
add(Ref, Index, Increment) ->
    ok = atomics:add(Ref, Index, Increment),
    Ref.

%% Add `Increment` to the atomic `Index` of `Ref` with `add_get/3`.
add_get(Ref, Index, Increment) ->
    _ = atomics:add_get(Ref, Index, Increment),
    Ref.

%% Exchange the atomic `Index` of `Ref` for `Value`.
exchange(Ref, Index, Value) ->
    _ = atomics:exchange(Ref, Index, Value),
    Ref.

%% Run `compare_exchange/4` on the atomic `Index` of `Ref`.
cas(Ref, Index, Expected, Desired) ->
    _ = atomics:compare_exchange(Ref, Index, Expected, Desired),
    Ref.

%% Write `Value` to the atomic `Index` of `Ref`.
put(Ref, Index, Value) ->
    ok = atomics:put(Ref, Index, Value),
    Ref.

%% Give the class and the reason of the exception of the call.
reason(Fun) ->
    try Fun() catch Class:Reason -> {Class, Reason} end.

%% Give the result of the call, or the class, the reason, and the first
%% frame of its exception, with each reference of the term as the atom
%% `reference`.
call(Fun) ->
    try Fun() of
        Result -> mask(Result)
    catch
        Class:Reason:Stack -> {Class, Reason, mask(hd(Stack))}
    end.

%% Give the term with each reference replaced by the atom `reference`, so
%% that a run of this module gives the same terms for each ERTS instance.
mask(Term) when is_reference(Term) -> reference;
mask(Term) when is_tuple(Term) ->
    list_to_tuple([mask(Element) || Element <- tuple_to_list(Term)]);
mask(Term) when is_list(Term) -> [mask(Element) || Element <- Term];
mask(Term) -> Term.
