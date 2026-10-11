# Copy-on-write terms

Status: design for the epic [Copy-on-write terms](https://github.com/fahchen/pbeam/issues/450). This document describes planned work. It is not an implementation claim. Documentation task: [#451](https://github.com/fahchen/pbeam/issues/451). The owner accepted this design on 2026-10-11 in Suikou review 01a1282e (design round 1, and the round 3 answers R3 and R4). The section [Owner decisions](#owner-decisions) lists each decision. A statement that this document marks as analysis is not an owner decision.

The [runtime specification](portable-beam-spec.md) defines the term model. GitHub Issues and the project board are the sources of task status.

## Goal

1. A clone of each compound term takes O(1) time: it only increments a reference count. A write copies a value only when another copy shares it (`Arc::make_mut`).
2. `Term` is `Send` and `Sync`, which the multi-scheduler work of M10 ([#10](https://github.com/fahchen/pbeam/issues/10), [#11](https://github.com/fahchen/pbeam/issues/11)) needs.
3. The observable behavior stays the same as on the pinned ERTS (OTP 29.1.1): return values, exceptions, messages, and reductions. Only the memory representation changes. This design does not change the reduction count rules.
4. Each step is one PR that can merge alone.

The specification already requires this model: "`Arc<Value>` for shared, immutable compound values" (`docs/portable-beam-spec.md:22`, `:95`), and a binary keeps the offset of a sub-binary (`docs/portable-beam-spec.md:97`). The code on main does not follow these lines. This design brings the code back to the specification.

Before this epic, #427 (PR #429) put `Map` behind `Rc<Repr>` with `Rc::make_mut`, and the key latency of the Terra Counter example became about 80 times shorter. #421 (PR #426) put the match context in `Rc<RefCell<MatchContext>>`. The other compound terms (tuple, list, binary, fun) are still copied deeply by each `Term::clone`, and the interpreter clones a term each time that it reads an operand.

## Current state

The file and line references are for revision `8a949de` of main.

| Location | Current state |
| --- | --- |
| `src/term.rs:35-81` | `Term` is an enum: `Binary(Vec<u8>)`, `List(Vec<Term>)`, `ImproperList(Vec<Term>, Box<Term>)`, `Tuple(Vec<Term>)`, `Map(Map)`, `Fun(Fun)`, and `MatchContext(Rc<RefCell<MatchContext>>)`. |
| `src/term.rs:209-232` | `shallow_clone`: only `Map` and `MatchContext` take O(1) time. `Binary` copies its `Vec<u8>`. A list, a tuple, and a fun give their nested terms to the caller, and the caller copies each one. |
| `src/term.rs:277-312` | `Clone for Term` copies the whole term with a work stack. It does not recurse, but it takes O(size) time. |
| `src/term.rs:244-275` | `take_nested` and `Drop` drop a term with a work stack, so a deep term does not overflow the Rust stack. |
| `src/term/map.rs:106-113` | `Map { repr: Rc<Repr> }`. `map.rs:308` and `:397` use `Rc::make_mut`. A HAMT `Node` holds its children by value (`map.rs:142-160`), so an update of a shared map copies the whole HAMT. |
| `src/term/map.rs:490-512` | `drain_into` uses `Rc::get_mut` to find the last holder. Only the last holder moves the nested terms to the drop work stack. |
| `src/term/match_context.rs:41`, `:63-69` | The bytes of a match context are an `Arc<[u8]>`, but `MatchContext::new` copies them from a `&[u8]`. |
| `src/term/fun.rs:231-236` | `Fun { target, free: Vec<Term> }`: a clone copies the free variables. |
| `src/term/atom.rs:38-44` | `Atom { index: u32, name: String }`: a clone of an atom allocates a `String`. |
| `src/runtime/interpreter.rs:3638-3666` | `read_operand` clones a literal (`:3644`) and an X or Y register (`:3651-3656`). Thus `element/2`, `hd/1`, `move`, and each BIF argument copy the term deeply each time. |
| `src/runtime/interpreter.rs:986-990`, `:2493` | `get_map_elements` reads its map through `map_values`, which reads the map operand with a copy. |
| `src/runtime/interpreter.rs:2020` | `call_fun` copies each free variable into an X register. |
| `src/runtime/interpreter.rs:2663-2681` | `nonempty_list`: `get_list` and `get_tl` use `values.remove(0)` and build the tail again, in O(n) time. Thus a recursive walk of a list takes O(n²) time. |
| `src/runtime/ets.rs:59`, `src/runtime/interpreter/bif/ets.rs:416`, `:519` | An ETS table stores `Vec<Vec<Term>>`. An insert clones each object into the table, and a lookup clones the object out of it. |
| `src/runtime/persistent_term.rs:91`, `:101`, `:132` | `persistent_term` stores `Term` values and clones them on each write and read. |
| `src/runtime/process.rs:490-512` | `process_info/2` gives `None` for `memory`, `heap_size`, and `binary`. |
| `src/runtime/interpreter/bif/table.rs:47`, `:344`, `:350`, `:359`, `:361` | `binary:referenced_byte_size/1`, `erts_debug:copy_shared/2`, `erts_debug:flat_size/1`, `erts_debug:same/2`, and `erts_debug:size_shared/1` are `unsupported`. |
| `docs/erts-differences.md:11-12` | Rows "Heap limit" and "Garbage collection reductions": PBeam has no process heap and no garbage collection reductions. |
| `docs/backlog.md:72-78` | The entry "Model the process heap for GC reductions and the message copy cost" owns the send copy reductions. |
| `docs/backlog.md:243-248` | The entry "Build a list cons and a map update without a copy": a cons takes O(n) time, and an update of a shared map takes O(n) time. |

## ERTS behavior

The file and line references are for the `OTP-29.1.1` tag. The probes ran on the pinned ERTS through `mise run erts-eval`.

- In a process, all terms are on the process heap. `move` and a register copy copy one boxed pointer. They never copy the nested terms.
- A local send copies the message to the heap of the receiver: `erl_message.c:834-861` calls `size_object_litopt` and `copy_struct_litopt`. The sharing-preserving copy (`SHCOPY_SEND`) exists only when `configure` gets `--enable-sharing-preserving` (`erts/configure.ac:874-882`, `global.h:1052-1055`). The default build does not have it, so the copy expands shared subterms. Probe: for `S = [L, L, L, L]` with `L = lists:seq(1, 1000)`, `erts_debug:size(S)` is 2008 in the sender and 8008 in another process that receives `S`. The copy skips literals (`erts_is_literal`, `copy.c:105-107`, `:119-121`).
- A send to the process itself with the message queue on the heap does not copy: `erl_message.c:824-826` sets `msize = 0`. Probe: `erts_debug:same(L, M)` is `true` for the received message `M`.
- Send reductions: `erl_message.c:881-886` adds one reduction for each 64 copied words (`ERTS_MSG_COPY_WORDS_PER_REDUCTION`, `erl_message.h:32-36`), with `CONTEXT_REDS` as the maximum. A send to the process itself has `msize = 0` and adds none.
- Binaries: a binary of at most 64 bytes is a heap binary (`ERL_ONHEAP_BINARY_LIMIT`, `erl_bits.h:166-167`), and a copy copies its bytes. A larger binary is a refc binary: a copy only increments its count (`erts_refc_inc`, `copy.c:812-842`). A sub-binary refers to its original binary (`binary_referenced_byte_size_1`, `erl_bif_binary.c:2508`).
- Map update: `erts_maps_put` (`erl_map.c:2105`) copies a whole flatmap (at most 32 pairs) and changes to a hashmap at 33 keys (`erl_map.c:2165`). For a hashmap, `erts_hashmap_insert` (`erl_map.c:2477`) copies only the nodes on the path, in O(log N) time. `maps:merge/2` returns one argument when both arguments are the same term (`erl_map.c:1257-1263`), and takes a subtree that both maps share with no step (`erl_map.c:1608-1612`).
- `setelement/3` copies the tuple (`bif.c:2976-3001`). The `set_tuple_element` instruction changes a new tuple in place.
- ETS: an insert copies the object into the table (`db_store_term`, `erl_db_util.c:3559`), and a lookup copies it out (`db_copy_from_comp`, `erl_db_util.c:3729`). The insert copy expands shared subterms (`size_object`, `erl_db_util.c:3579`, and `copy_ets_element`, `:3613`). Probe: two `ets:lookup/2` results of one object are not `erts_debug:same/2`. For `S = [L, L, L, L]`, `erts_debug:size/1` of the looked-up `S` is 8008.
- `persistent_term`: `put/2` copies the term to a literal area with the sharing-preserving copy (`erl_bif_persistent.c:744-761`). `get/1` does not copy (`erl_bif_persistent.c:380-388`). Probe: two `persistent_term:get/1` results of one key are `erts_debug:same/2`, and the result is not `erts_debug:same/2` as the term given to `put/2`. For `S = [L, L, L, L]`, `erts_debug:size/1` of the `get/1` result is 2008, as for `S`.
- The copy cost is observable through: the send reductions (above), `process_info/2` items `memory`, `heap_size`, and `binary`, `erts_debug:size/1` (`lib/kernel/src/erts_debug.erl:257-259`, which finds shared subterms with `erts_debug:same/2`, `:357-362`), `erts_debug:flat_size/1`, `erts_debug:size_shared/1`, `binary:referenced_byte_size/1`, `erlang:memory/1`, and the time and reductions of garbage collection (`gc_cost`, `erl_gc.c:218-230`). PBeam supports none of them on main (see [Current state](#current-state)). Thus physical sharing changes no result that PBeam can show now.

Probe commands:

```sh
LANG=en_US.UTF-8 mise run erts-eval 'L = lists:seq(1,1000), S = [L,L,L,L], Self = self(), P = spawn(fun() -> receive M -> Self ! {size, erts_debug:size(M)} end end), P ! S, R = receive {size, N} -> N end, self() ! L, Same = receive M2 -> erts_debug:same(L, M2) end, {erts_debug:size(S), R, Same, erlang:system_info(otp_release)}.'
# {2008,8008,true,"29"}
LANG=en_US.UTF-8 mise run erts-eval 'L = lists:seq(1,1000), persistent_term:put(cowk, L), T = ets:new(t, [set]), ets:insert(T, {k, L}), [{k, E1}] = ets:lookup(T, k), [{k, E2}] = ets:lookup(T, k), {erts_debug:same(persistent_term:get(cowk), persistent_term:get(cowk)), erts_debug:same(L, persistent_term:get(cowk)), erts_debug:same(E1, E2), erts_debug:same(E1, L)}.'
# {true,false,false,false}
LANG=en_US.UTF-8 mise run erts-eval 'L = lists:seq(1,1000), S = [L,L,L,L], T = ets:new(t, [set]), ets:insert(T, {k, S}), [{k, E}] = ets:lookup(T, k), persistent_term:put(cowk2, S), P = persistent_term:get(cowk2), {erts_debug:size(S), erts_debug:size(E), erts_debug:size(P)}.'
# {2008,8008,2008}
```

## Options

| Option | Content | Advantages | Disadvantages | Cost | Risk |
| --- | --- | --- | --- | --- | --- |
| A. `Arc` copy-on-write for all compound terms, lists as cons cells | A tuple is `Arc<Vec<Term>>`. A list is `Nil` or `Cons(Arc<Cons>)`. A binary is `Arc<[u8]>` with an offset and a length. A map and its HAMT nodes use `Arc`. A fun is `Arc<FunThing>`. A match context is `Arc` with an atomic state. | A clone takes O(1) time. `get_list` and `put_list` take O(1) time, as on ERTS. The canonical form rule of `ImproperList` goes away. `Term` is `Send + Sync`, which M10 needs. It follows the specification (`:22`, `:95-97`). | It changes the most code: at `8a949de`, `src/` has 321 matches of `Term::List(`, 58 of `ImproperList(`, 211 of `Tuple(`, and 187 of `Binary(`; `tests/` has 1111. `length/1` and `lists:nth/2` take O(n) time (as on ERTS). Each cons is one allocation. | High (about 8 to 10 PRs) | Medium: the list change is large, so an API layer comes first. |
| B. `Rc` copy-on-write, deep copy at the process boundary | The structure of A with `Rc`. A send and ETS copy deeply (the ERTS cost model). M10 adds `unsafe impl Send`. | No atomic cost. The send cost model is the same as on ERTS. | On ERTS, `persistent_term` and refc binaries are shared between processes; `Rc` cannot do this. `unsafe impl Send` needs a proof that only one process can reach each `Rc` of the process, and ETS, `persistent_term`, and timer messages break this proof. M10 changes the representation again. | Medium, then high again for M10 | High: the `unsafe` invariant is hard to keep. |
| C. `Arc` only for tuples, funs, and binaries; lists stay `Vec` | `List(Arc<Vec<Term>>)`. A cons still copies. | Small change. A clone takes O(1) time. | `get_list`, `get_tl`, and `put_list` still take O(n) time, and a recursive walk of a list still takes O(n²) time (`interpreter.rs:2663-2681`). The `ImproperList` rule stays. | Low to medium | Low, but the list hot spot stays. |

## Selected option

The owner selected option A (round 1). The work adds an API layer first and changes each representation after it.

1. Use `Arc` now, not `Rc`. A clone and drop of `Arc` costs about 5 ns more than `Rc` (see [Measurements](#measurements)), but the deep copy on main costs microseconds to milliseconds. Refc binaries and `persistent_term` share terms between processes. M10 (#10, #11) needs `Process: Send`. With `Rc`, M10 must change all terms again.
2. A send, ETS, and `persistent_term` share a term physically (O(1)). See [Sharing between processes](#sharing-between-processes).
3. A list is made of cons cells. `get_list` and `put_list` take O(1) time, as on ERTS. An improper list has its natural form, and the canonical form rule of `ImproperList` goes away. A list API comes first (#457), so callers do not match a `Vec`. Then one PR changes the representation (#458).
4. Drop keeps the work stack and uses `Arc::into_inner` for each `Arc` node. See [Drop rule](#drop-rule).
5. A match context is `Arc<MatchState>`. Its bytes are a shared binary (no copy in `MatchContext::new`). Its position and its `position_known` flag are in one `AtomicU64`: 63 bits for the bit position and 1 bit for the flag. Only the owner process uses it, so `Relaxed` order is sufficient, and it needs no `RefCell` and no `Mutex`.
6. `PartialEq` has an `Arc::ptr_eq` fast path. A compile-time assertion checks `Term: Send + Sync`.
7. A map and its HAMT nodes are in `Arc`. An update copies only the path (O(log N)), as `erl_map.c:2477` does. A flatmap of at most 32 pairs is copied whole, as `erts_maps_put` does. Shared subtrees make the steps of `maps:merge/2` the same as on ERTS (`erl_map.c:1608-1612`). This is a reduction change, so #453 measures it against the pinned ERTS. Result of #453: the steps are the same as on ERTS for a map and an update of it. They are different for the results of the other map BIFs and for a map from a send, ETS, or `persistent_term` (row "Map merge reductions" of `docs/erts-differences.md`).

Expected result (analysis): the tuple, list, nested, and send benchmarks go from milliseconds to microseconds. `size_of::<Term>()` goes from 96 bytes to at most 24 bytes after #460. See [Limits and open points](#limits-and-open-points) for the binary layout that this target needs.

## Representation of each term kind

| Kind | On main (`8a949de`) | Planned | Write rule | Task |
| --- | --- | --- | --- | --- |
| Integer, pid, port, reference | `i64` or `u64` in the enum | No change | Immediate | None |
| Atom | `Atom { index: u32, name: String }` | No inline `String`: for example an `Arc<str>`, or only the index | Immutable; a clone does not allocate | #460 |
| Tuple | `Tuple(Vec<Term>)` | `Tuple(Option<Arc<Vec<Term>>>)` newtype with `Deref<Target = [Term]>` and the `Term::tuple` constructor (#454) | `set_tuple_element` and `setelement/3` use `Arc::make_mut`: a shared tuple is copied (O(n), as `setelement/3` on ERTS), an unshared tuple changes in place | #454 |
| List | `List(Vec<Term>)` and `ImproperList(Vec<Term>, Box<Term>)` | `Nil` or `Cons(ConsRef)`, where a `ConsRef` holds an `Arc<Cons>` (see [Drop rule](#drop-rule)); a cons holds a head and a tail; `ImproperList` is removed (#458) | `put_list` makes a new cons in O(1) time and shares the tail; `get_list`, `get_hd`, and `get_tl` read the cell in O(1) time. The copy of the head or the tail into a register is O(1) for a list, a map, or an immediate, but a tuple, a fun, or a binary is still a deep copy until #454, #455, and #456 | #457 (API), #458 (cons cells) |
| Binary | `Binary(Vec<u8>)` | `Arc<[u8]>` with an offset and a length | The bytes do not change after construction. `binary_part/3`, `bs_get_binary2`, `bs_get_tail`, and `binary:split/2,3` return views of the same bytes | #456 |
| Map | `Map { repr: Rc<Repr> }`; HAMT nodes by value | `Arc<Repr>`; each HAMT node in `Arc` | `Arc::make_mut` on each node of the path; a flatmap is copied whole | #453 |
| Fun | `Fun { target, free: Vec<Term> }` | `Fun(Option<Arc<FunThing>>)` newtype, where `FunThing` holds the entry and the free variables (#455) | Immutable; a copy shares the thing, and the entry and the free variables do not change | #455 |
| Match context | `Rc<RefCell<MatchContext>>`; bytes copied into an `Arc<[u8]>` | `Arc<MatchState>`; bytes are the shared binary; position and `position_known` in one `AtomicU64` | `Relaxed` atomic store by the owner process | #456 (bytes), #459 (`Arc` and atomic) |

After #454, #455, #456, and #458, `Term::clone` is a shallow copy. #455 removes the work stack of `Clone`: `shallow_clone` keeps the `Term` variants and the byte copy of a binary, and `Clone::clone` calls it. #456 changes the binary arm of `shallow_clone`, and #459 removes `shallow_clone`. Only the work stack of `Drop` stays.

## List API

Task #457 added the list API in `src/term/list.rs`, and #458 changed the representation to cons cells. A list is `Term::Nil` or `Term::Cons(ConsRef)`. A `ConsRef` holds the `Arc` of a `Cons`, which has a head and a tail (see [Drop rule](#drop-rule)). Code outside the term module reads and builds a list only through these functions of `Term`, and it does not match `Term::Nil` or `Term::Cons`. Only `src/term.rs` and the files in `src/term/` match these variants.

| Function | Use | Time |
| --- | --- | --- |
| `nil()` | Build `[]`. | O(1) |
| `cons(head, tail)` | Build one cons cell that shares `tail` (`put_list`). | O(1) |
| `list_from(elements)` | Build a proper list from an iterator. | O(n) |
| `list_with_tail(elements, tail)` | Build a cons cell for each element, with `tail` after the last one. | O(n) |
| `is_nil()`, `is_list()`, `is_nonempty_list()` | Test for `[]`, for `[]` or a cons cell, and for a cons cell, as the `is_nil`, `is_list`, and `is_nonempty_list` instructions do. | O(1) |
| `cons_cell()` | Read the head and the tail of a cons cell (`get_list`, `get_tl`, `tl/1`). | O(1) |
| `list_head()` | Read the head of a cons cell (`hd/1`, `get_hd`). | O(1) |
| `list_iter()` | Walk the elements of a list. `ListIter::rest` gives the part that the walk did not read: a cons cell, `[]`, or the tail of an improper list. | O(1) to start |
| `list_cells()` | Walk a list to its end, and give an iterator of its elements and the tail of an improper list. | O(n) |
| `proper_list()` | Walk a list to its end, and give an iterator of its elements when the list is proper. | O(n) |
| `list_elements()` | Copy the elements of a list into a vector for an apply, and tell whether the list is proper. | O(n) |

A trap of a list BIF keeps the rest of its list as a term in O(1) time: `length/1`, `lists:keyfind/3`, `lists:keymember/3`, `lists:member/2`, and `lists:reverse/2` trap with `ListIter::rest`, and `++` traps to `erlang:'++'/3` with the original lists and the rest of the left list, as the ERTS append state keeps its iterator (`erl_bif_lists.c:47-55`). The iodata walk and the chardata walks keep the rest of each list on their stacks. The reductions and the chunk sizes do not change.

## Drop rule

- `Drop` keeps the work stack, so a deep term or a long list does not overflow the Rust stack.
- For each `Arc` node, `Drop` calls `Arc::into_inner`. When it returns the inner value, this holder was the last one, and the nested terms go on the work stack. When it returns `None`, another holder keeps the node, and nothing goes on the stack.
- A cons cell puts its tail and then its head on the work stack, so the head comes off first, and the stack stays short for a long list. `Term` has a `Drop`, so a drop cannot move the `Arc` out of a `Term::Cons`. Thus `Term::Cons` holds a `ConsRef`, a wrapper of `Option<Arc<Cons>>`: the drop takes the `Arc` out of the `Option` and calls `Arc::into_inner`. The `Option` is empty only after this, and every other reader finds a cell. A replacement `Arc` would need one more allocation for each dropped cell (#458).
- A tuple holds its elements in `Tuple(Option<Arc<Vec<Term>>>)` for the same reason: the drop takes the `Arc` out of the `Option` and calls `Arc::into_inner`, and the elements of the last holder go on the work stack (#454).
- A fun holds its thing in `Fun(Option<Arc<FunThing>>)` for the same reason: the drop takes the `Arc` out of the `Option` and calls `Arc::into_inner`, and the free variables of the last holder go on the work stack (#455).
- `Arc::into_inner` is in `impl<T, A: Allocator> Arc<T, A>`, which has an implicit `T: Sized` bound. Thus each `Arc` that holds terms has a sized payload. A tuple is `Arc<Vec<Term>>`, not `Arc<[Term]>`: `Arc::into_inner` does not accept `Arc<[Term]>`. `Arc::make_mut` accepts `Arc<[Term]>` and returns a `&mut [Term]`, so it is not a second reason. The cost is a second allocation for each tuple and one more pointer step for each element read. A thin `Arc` of a header and an inline slice needs `unsafe` code, so this design does not use it. A binary `Arc<[u8]>` holds no terms, so its usual drop does not recurse.
- Do not use `Arc::get_mut` or `Arc::try_unwrap` for this. When two threads drop the last two holders at the same time, both calls can fail, and the drop of the last `Arc` then drops the inner value recursively. The standard library guarantees that, of concurrent `Arc::into_inner` calls on all clones, exactly one call returns the inner value.
- `Map::drain_into` changes from `Rc::get_mut` to `Arc::into_inner` in #453.

## Equality and thread safety

- `PartialEq` returns `true` for two terms that hold the same `Arc` (`Arc::ptr_eq`) before it compares their contents. Two different allocations with equal contents are still compared element by element, so the worst case stays O(size). #454 adds the fast path for a tuple, #458 for a cons cell, and #455 for a fun. The fast path of a fun compares its shared thing, which holds the entry and the free variables.
- A compile-time assertion checks `Term: Send + Sync` (#459). On main, `Rc<Repr>` of `Map` (`src/term/map.rs:112`) and `Rc<RefCell<MatchContext>>` (`src/term.rs:80`) are the parts that are not `Send`. #453 replaces the first, and #459 replaces the second.
- Process mutable state stays exclusive to its process. Terms are shared and immutable. #459 changes the wording of #10 to this rule.

## Sharing between processes

A send, an ETS insert or lookup, and `persistent_term` share a term physically: they increment a reference count and do not copy. A receiver cannot see a difference, because terms are immutable, and PBeam reports no value that depends on a physical copy (see [ERTS behavior](#erts-behavior)).

Logical copy rule: a later implementation of a value that depends on a copy computes the value from the ERTS model, not from the `Arc` counts of PBeam. For `memory`, `heap_size`, `erts_debug:size/1`, and the send reductions, it computes the logical copy size with a walk that does not allocate, as `size_object_litopt` does: a send to another process expands shared subterms, and a send to the process itself with the message queue on the heap copies nothing. Analysis of this document: the same rule applies to `erts_debug:same/2` and `binary:referenced_byte_size/1`. They must give the ERTS result, not a result from `Arc::ptr_eq` or the `Arc` allocation. For example, `erts_debug:same/2` of a term and its copy in another process is `false` on ERTS, and `binary:referenced_byte_size/1` follows the ERTS heap binary and refc binary kinds.

- `docs/erts-differences.md` gets a "Message copy" row in #459: PBeam shares a term in a send, ETS, and `persistent_term`. The row states the ERTS copy policy of each operation: a send to another process copies the term and expands shared subterms, and a send to the process itself with the message queue on the heap does not copy; an ETS insert copies the term and expands shared subterms, and a lookup copies the stored term out; `persistent_term:put/2` copies the term and keeps its internal sharing (`erl_bif_persistent.c:744-761`), and `get/1` does not copy. The row states the logical copy rule.
- The backlog entry "Model the process heap for GC reductions and the message copy cost" keeps the send reductions.
- PBeam does not track the literal origin of a term now. Only the send reductions or `memory` need it (the copy skips literals, `copy.c:105-107`), and the same backlog entry owns it.

## Needless clones

Apart from shared terms, the interpreter clones operands that it only reads. #452 adds a borrowed read, for example `read_ref(&Operand) -> &Term`, for literals and registers. Read-only instructions use it, and an instruction clones a term only when it stores the term in another place. Known cases on main: `read_operand` clones the X and Y registers (`interpreter.rs:3651-3656`); `get_map_elements` copies the map through `map_values` to read fields (`interpreter.rs:986-990`, `:2493`); `get_tuple_element`, the type tests, the comparisons, and the free-variable read of `call_fun` (`interpreter.rs:2020`) also clone. The PR audits `.clone()`, `to_vec()`, and `to_owned()` with `clippy::redundant_clone` and a grep. #452 is the first implementation task, because it is independent and gives the fastest gain.

## Measurements

### Interpreter benchmarks

Revision `8a949de`, release build, Apple Silicon macOS, 2026-10-11. Each value is the median of three runs in microseconds. The benchmark module is not committed (owner decision): save the module of [Benchmark module](#benchmark-module) as `/tmp/cow451/cowb.erl`.

```sh
mkdir -p /tmp/cow451
LANG=en_US.UTF-8 mise exec -- erlc -o /tmp/cow451 /tmp/cow451/cowb.erl
LANG=en_US.UTF-8 mise exec -- cargo build --release --locked
# ERTS, for each function (send, send2, tuple, list, map_get, map_put, nested):
LANG=en_US.UTF-8 mise run erts-eval --pa /tmp/cow451 "spawn_link(cowb, tuple, []), receive {'EXIT', _, R} -> R end."
# PBeam, for each function:
target/release/pbeam cowb tuple -- /tmp/cow451/cowb.beam
```

| Benchmark | ERTS µs | PBeam µs | Ratio |
| --- | --- | --- | --- |
| `send`: send a 1000-element list to the process itself, 2000 times | 174 | 169152 | about 970 |
| `send2`: send a 1000-element list to another process, 2000 times | 10315 | 154580 | about 15 |
| `tuple`: `element/2` on a 1000-element tuple, 20000 times | 54 | 299322 | about 5500 |
| `list`: `hd/1` on a 1000-element list, 20000 times | 39 | 314666 | about 8100 |
| `map_get`: `map_get/2` on a 1000-key map, 20000 times | 132 | 27142 | about 210 |
| `map_put`: `M#{N => N}` that keeps `M`, 2000 times | 146 | 125324 | about 860 |
| `nested`: `element/2` on a tuple of 100 lists of 100 elements, 2000 times | 5 | 298058 | about 60000 |

The ERTS values below 100 µs can vary by more than 20% between runs. The design run of the same day on an earlier main gave ratios within a factor of 2 of these ratios. The revision of this table is `8a949de`, before #452 and #453, so its PBeam values are higher than the values of the next section.

### Tuple benchmark (#454)

Release build, Apple Silicon macOS, revision `901beff` before the tuple change and the branch of #454 after it. Each value is the median of five runs in microseconds, with no other load on the host. The benchmark module of [Benchmark module](#benchmark-module) is at `/tmp/cow454/cowb.erl`. The commands are the commands above, with the directory `/tmp/cow454` and the revision before the change in a second worktree.

| Benchmark | ERTS µs | Before µs | After µs |
| --- | --- | --- | --- |
| `tuple`: `element/2` on a 1000-element tuple, 20000 times | 41 | 316007 | 20886 |
| `nested`: `element/2` and `hd/1` on a tuple of 100 lists of 100 elements, 2000 times | 6 | 6555 | 3191 |
| `list`: `hd/1` on a 1000-element list, 20000 times | 53 | 18638 | 17992 |
| `send`: send a 1000-element list to the process itself, 2000 times | 174 | 2921 | 3024 |
| `send2`: send a 1000-element list to another process, 2000 times | 10756 | 4870 | 5032 |
| `map_get`: `map_get/2` on a 1000-key map, 20000 times | 216 | 24124 | 25218 |
| `map_put`: `M#{N => N}` that keeps `M`, 2000 times | 146 | 4942 | 4861 |

The `tuple` benchmark is about 15 times faster and the `nested` benchmark about 2 times faster. The four other benchmarks stay within the run-to-run spread, because they do not copy a tuple.

A second probe of the same day isolates the remaining `tuple` time: `tuple_size/1` on the same tuple, 20000 times, gives 26414, 28351, and 28417 µs after the change and 34, 35, and 40 µs on ERTS. Thus the remaining ~21 ms of the `tuple` benchmark is the recursion, the arithmetic, and the BIF call of the loop, not the tuple read. The module `/tmp/cow454/cowb3.erl` (not committed) holds the probe.

### Fun benchmark (#455)

Release build, Apple Silicon macOS, revision `31f765b` (origin/main) before the fun change and the branch of #455 after it. Each value is the median of five interleaved runs in microseconds. The host was shared with other work, so the spread is wide. Each `after` value of the two probes below is lower than each `before` value of the same probe. The probe module is the [Benchmark module](#benchmark-module) at `/tmp/dsh/cow455/cowb.erl`, and the commands are the commands above with that directory and the revision before the change in a second worktree.

| Probe | ERTS µs | Before µs | After µs |
| --- | --- | --- | --- |
| `funcopy`: put a fun with a 1000-element free variable into a tuple, 20000 times | 34 | 13734 | 12971 |
| `funsend`: send that fun to the process itself, 2000 times | 178 | 4807 | 3061 |

The design benchmarks (`tuple`, `list`, `send`, `send2`, `map_get`, `map_put`, and `nested`) do not copy a fun. Their medians moved within the spread of the shared host, which is wider than the effect of this change, so this run does not separate them. #454 and #458 already made the nested terms of a fun an O(1) copy. Thus the change removes the free-variable vector and the copy of the entry of an unshared fun. The remaining time of `funcopy` and `funsend` is the BIF call, the tuple or message work, and the loop.

### Rust micro-benchmark

Revision `8a949de`, release build. A scratch crate (not committed) depends on `pbeam-runtime` by path and runs `cargo run --release`. It prints `size_of::<Term>()`, the clone and drop time of `Rc` and `Arc`, and the time of `Term::clone` and `Arc::clone` of a 1000-element tuple of integers.

| Value | Result |
| --- | --- |
| `size_of::<Term>()`, `size_of::<Atom>()`, `size_of::<Map>()` | 96, 32, 8 bytes |
| `Rc` clone and drop | 4.51 ns |
| `Arc` clone and drop | 9.90 ns |
| `Term::clone` of a 1000-element tuple | 13474 ns |
| `Arc::clone` of the same elements | 9.85 ns |

The atomic cost of `Arc` (about 5 ns for each clone) is small compared with the deep copy on main.

### Terra Counter key latency

The design run of 2026-10-11 on main (revision not recorded; this task did not run it again): start the Terra Counter example in a PTY, wait 20 s, send the keys `j`, `j`, `k`, and `q` 2 s apart, and record the time from each key to the first output after it. The output came 0.13, 0.16, 0.17, and 0.21 s after the four keys. Before #427, each output came seconds after its key. The PTY script of this run is not in the repository.

### Benchmark module

```erlang
-module(cowb).
-export([send/0, send2/0, sink/2, tuple/0, list/0, map_get/0, map_put/0, nested/0, funcopy/0, funsend/0]).

%% Each benchmark exits with {Name, Microseconds}.
time(Name, F) ->
    T0 = erlang:monotonic_time(microsecond),
    _ = F(),
    R = {Name, erlang:monotonic_time(microsecond) - T0}, exit(R).

seq(N, N) -> [N];
seq(I, N) -> [I | seq(I + 1, N)].

%% Self-send of a 1000-element list, 2000 times.
send() -> L = seq(1, 1000), time(send, fun() -> send_loop(L, 2000) end).
send_loop(_, 0) -> ok;
send_loop(L, N) -> self() ! L, receive _ -> send_loop(L, N - 1) end.

%% element/2 on a 1000-element tuple, 20000 times.
tuple() -> T = list_to_tuple(seq(1, 1000)), time(tuple, fun() -> tuple_loop(T, 20000, 0) end).
tuple_loop(_, 0, A) -> A;
tuple_loop(T, N, A) -> tuple_loop(T, N - 1, A + element(500, T)).

%% hd/1 on a 1000-element list, 20000 times.
list() -> L = seq(1, 1000), time(list, fun() -> list_loop(L, 20000, 0) end).
list_loop(_, 0, A) -> A;
list_loop(L, N, A) -> list_loop(L, N - 1, A + hd(L)).

%% map_get/2 on a 1000-key map, 20000 times.
map_get() -> M = maps:from_list([{K, K} || K <- seq(1, 1000)]), time(map_get, fun() -> mg_loop(M, 20000, 0) end).
mg_loop(_, 0, A) -> A;
mg_loop(M, N, A) -> mg_loop(M, N - 1, A + map_get(500, M)).

%% M#{K => V} on a 1000-key map that the caller keeps, 2000 times.
map_put() -> M = maps:from_list([{K, K} || K <- seq(1, 1000)]), time(map_put, fun() -> mp_loop(M, 2000) end).
mp_loop(_, 0) -> ok;
mp_loop(M, N) -> put(m, M#{N => N}), mp_loop(M, N - 1).

%% element/2 on a tuple of 100 lists of 100 elements, 2000 times.
nested() -> T = list_to_tuple([seq(1, 100) || _ <- seq(1, 100)]), time(nested, fun() -> nested_loop(T, 2000, 0) end).
nested_loop(_, 0, A) -> A;
nested_loop(T, N, A) -> nested_loop(T, N - 1, A + hd(element(50, T))).

%% Put a fun with a 1000-element free variable into a tuple, 20000 times.
funcopy() -> A = seq(1, 1000), F = fun() -> A end, time(funcopy, fun() -> fc_loop(F, 20000) end).
fc_loop(_, 0) -> ok;
fc_loop(F, N) -> T = {F, N}, fc_loop(element(1, T), N - 1).

%% Self-send of a fun with a 1000-element free variable, 2000 times.
funsend() -> A = seq(1, 1000), F = fun() -> A end, time(funsend, fun() -> fs_loop(F, 2000) end).
fs_loop(_, 0) -> ok;
fs_loop(F, N) -> self() ! F, receive _ -> fs_loop(F, N - 1) end.

%% Send of a 1000-element list to another process, 2000 times.
send2() -> L = seq(1, 1000), time(send2, fun() -> Self = self(), P = spawn(cowb, sink, [Self, 2000]), send2_loop(P, L, 2000), receive done -> ok end end).
send2_loop(_, _, 0) -> ok;
send2_loop(P, L, N) -> P ! L, send2_loop(P, L, N - 1).
sink(From, 0) -> From ! done;
sink(From, N) -> receive _ -> sink(From, N - 1) end.
```

## Task order

Each task is one PR. The native blocked-by links of the issues are the source of the order.

1. [#451](https://github.com/fahchen/pbeam/issues/451) Record the copy-on-write term design (this document). No blocker.
2. [#452](https://github.com/fahchen/pbeam/issues/452) Read operands by reference in read-only instructions. No blocker; it can run in parallel with #451.
3. [#453](https://github.com/fahchen/pbeam/issues/453) Share map trie nodes between the copies of a map. Blocked by #451 and #452.
4. [#454](https://github.com/fahchen/pbeam/issues/454) Share a tuple between the copies of a term. Blocked by #453 (it uses the drop and `ptr_eq` pattern of #453).
5. [#455](https://github.com/fahchen/pbeam/issues/455) Share the free variables of a fun. Blocked by #454.
6. [#456](https://github.com/fahchen/pbeam/issues/456) Share binary bytes between binaries, sub-binaries, and match contexts. Blocked by #454. It does not add `binary:referenced_byte_size/1` (no consumer).
7. [#457](https://github.com/fahchen/pbeam/issues/457) Read and build lists through one list API. Blocked by #451 and #452; it can run in parallel with #453 to #456. It can split into sub-issues by module (interpreter, BIFs, runtime).
8. [#458](https://github.com/fahchen/pbeam/issues/458) Store a list as cons cells. Blocked by #457.
9. [#459](https://github.com/fahchen/pbeam/issues/459) Make terms Send and Sync and share them between processes. Blocked by #454, #455, #456, and #458. #10 is blocked by this task.
10. [#460](https://github.com/fahchen/pbeam/issues/460) Clone an atom without a heap allocation. Blocked by #454.

The backlog entry "Build a list cons and a map update without a copy" went away in two parts: #453 removed the map part, and #458 removed the rest.

## Owner decisions

These are the decisions of 2026-10-11 in Suikou review 01a1282e.

- Option A, with `Arc` now (design round 1; design question 1).
- Lists are cons cells, not `Vec` (design question 2).
- A send shares a term (R3; design question 3). #10 changes its wording to "process mutable state is exclusive; terms are shared and immutable".
- The epic is `[m5-e4]` in the milestone `[m5] Interactive host I/O` (R4; design question 8). The design draft named it `[m5-e3]`; the code loading epic took that number. The first implementation task is the borrowed read (#452).
- The mutable state of a match context is an `AtomicU64` with `Relaxed` order, not a `Mutex` (design question 4).
- #453 measures the steps of `maps:merge/2` against the pinned ERTS (design question 5). The row "Map merge reductions" of `docs/erts-differences.md` records the result.
- PBeam does not track the literal origin of a term now (design question 6).
- No benchmark is committed. Each PR records the measurement commands and results (design question 7).

## Limits and open points

- Analysis of this document: the target of #460, `size_of::<Term>()` at most 24 bytes, needs each enum payload to be at most 16 bytes. A fat `Arc<[u8]>` (16 bytes) with an inline `usize` offset and length is 32 bytes. Thus #456 keeps the offset and the length behind one pointer (for example an `Arc` that holds a sub-binary header), or #460 changes its target. A smaller offset or length type adds a binary size limit that ERTS does not have, so it is not permitted.
- `length/1` and `lists:nth/2` take O(n) time on cons cells, as on ERTS. Since #458, code that needs the length of a list walks it.
- The drop of the last holder of a term still takes O(size) time, as on main.
