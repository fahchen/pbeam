-module(atomics_probe).
-export([main/0, allocation_failure/0, write_concurrency/0,
         write_concurrency_allocation_failure/0, atomics_only/0]).

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

%% The `atomics` functions that the `counters` module does not call, which
%% PBeam does not implement. Only a PBeam run calls it.
atomics_only() ->
    Ref = atomics:new(1, []),
    [atomics:add_get(Ref, 1, 1), atomics:exchange(Ref, 1, 1)].

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
