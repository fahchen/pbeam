# Term storage and interpreter performance

Status: revised proposal dated 2026-10-13 after the local second design (`term-performance-design-deepseek.md`, not part of this PR). This local revision uses the owner-supplied `erlang_immutable_data_structures_pbeam_reference.md`, dated 2026-10-13. No implementation starts. Task [#459](https://github.com/fahchen/pbeam/issues/459) is merged on main through [#496](https://github.com/fahchen/pbeam/pull/496) at `a6b838c`. Task #456 is also merged through #495. The measurement revision stays `a6b838c` even if another document changes while this proposal is written. Terms are now `Send + Sync`, and shared storage uses `Arc`. The owner prefers to keep `Arc` and avoid a collector. This proposal follows that direction.

Status: proposed successor to [Copy-on-write terms](copy-on-write-terms.md). This proposal does not replace its accepted owner decisions. It does not authorize implementation. It is based on the user measurements at `31f765b` and source inspection and layout checks at `a6b838c`. The toolchain pin is OTP 29.1.1. GitHub Issues remain the delivery record.

## Result to seek

Reduce the Terra redraw from 82 ms to at most 4 ms on the same host. The first target is 10 ns per original BEAM instruction, or 2.8 ms for 280000 instructions before other work. The stretch target is 3 ns per instruction, or 0.84 ms. These are acceptance targets, not measured results. The supplied historical 155 ns per instruction gives 43.4 ms for that walk. An 8-byte term alone cannot close this gap.

Keep the current safe enum and `Arc` storage for the first dispatch changes. A safe 16-byte term remains a later layout target, subject to a measured gain. Use shared immutable compound values. Resolve BIFs and branches at load time. Keep the active process, frame, module, and instruction position in the execution loop. Use fixed-arity borrowed BIF arguments. Add short combined instructions only after these changes have measured results. Retain the original BEAM positions and reduction switch points.

The ERTS reference can use its JIT. Its native arithmetic code does not have an interpreter dispatch for each instruction. The small-integer special cases are in [instr_arith.cpp:145–176](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/jit/arm/instr_arith.cpp#L145). A portable Rust interpreter has a lower speed ceiling. If the stretch target needs native code, that work needs a separate scope decision because JIT and AOT are outside the current runtime specification.

## Evidence and scope

The historical supplied bare loop costs 505–512 ns per iteration for three instructions. The added BIF costs are about 297–307 ns. A direct tuple read adds 148 ns. A map read adds 633 ns, but its HAMT walk takes only 34 ns. These results identify generic dispatch and execution state access as the first work. The equal costs of calls with 1-element and 1000-element tuples already rule out deep tuple copying on that path.

The historical binary copy cost was 19.7 us for 100 KB before merged task #456. The inferred rate is about 5 GB/s. The baseline must now check that merged task #456 removed the size slope. It cannot remove the 505 ns integer loop cost. The 0.33 send2 ratio does not establish better general send performance because the current physical copy rules differ.

At the historical inspection revision `a6b838c`, source already used shared tuple arrays, fun objects, cons cells, atom names, and map nodes. `Instruction::Bif` in `src/runtime/interpreter.rs` still builds a `Vec<Term>`, checks the BIF name, and calls generic dispatch. The run loop still gets the active process and frame and compares module names for each instruction. Operand access still checks an operand variant and register state. `Term::Drop` creates a work vector even for an immediate. These are historical source findings, not measured shares of the 155 ns cost. They do not describe current main. At `57b6629`, immediate `Term::Drop` takes the empty match arm, and equality uses an inline pair stack (`src/term.rs`). Do not repeat work that has already merged. Use [the dispatch measurements](interpreter-dispatch-measurements.md) and [the data structure audit](data-structure-audit.md) to select the next measured consumer.

The 96-byte term in the brief is a historical value. The second design reports 32 bytes at `b52e297` after the fun change. The current layout check gives 40 bytes after the binary view change. Measure `Term`, `Option<Term>`, the register slot, each compound header, and each decoded instruction at the baseline. Do not use 96 bytes as the current register width.

The second design measured layouts at `b52e297`, not at current main. I ran `mise exec -- env RUSTC_BOOTSTRAP=1 cargo rustc --lib -- -Zprint-type-sizes` with pinned Rust 1.99.0 at `a6b838c` on arm64. The following table compares its reported sizes with my current results. I did not rebuild `b52e297`, so the old results are attributed to the second design. Equal current sizes confirm only the current layout, not the old measurement procedure.

| Type | Second design at `b52e297`, bytes | Verified at `a6b838c`, bytes |
| --- | --- | --- |
| `Term` | 32 | 40 |
| `Option<Term>` | Not reported | 40 |
| `Atom` | 24 | 24 |
| `Operand` | 32 | 40 |
| `Instruction` | 112 | 136 |
| `StackSlot` | 32 | 40 |
| `Cons` | 64 | 80 |
| `map::Child` | 64 | 80 |
| `map::Hamt`, `map::Repr` | 40 | 40 |
| `map::Node` | 32 | 32 |
| `Tuple`, `Map`, `Fun` | 8 | 8 |
| `FunThing`, `Frame` | 80 | 80 |
| `ProcessState` | 840 | 848 |
| `mailbox::Node` | 104 | 112 |
| `mailbox::Entry` | 72 | 80 |

I also ran a temporary Rust release probe with `black_box` around each source handle and cloned result. It used five million clone/drop pairs per sample and nine samples. `Rc<u64>` took 4.258 to 4.300 ns per pair. `Arc<u64>` took 9.986 to 10.162 ns per pair. The minimum difference is 5.728 ns per pair. The host was arm64 with load averages 7.41, 9.95, and 16.48 after the run. These results support the direction of the second design's 3.7 ns and 9.7 ns results. They do not reproduce its exact experiment or measure contended sharing. Its stated 5.6 ns difference is inconsistent with its printed numbers: 9.7 minus 3.7 is 6.0 ns.

I did not verify its word-copy, enum-width, register, allocation, string, BIF-search, process-table, label-scan, or tagged-store timings. No probe source or release assembly for those results is in the second document. Its 460 to 520 ns cost budget is a hypothesis, not a profile: isolated timings can overlap and do not establish the cost of the real instruction path. The 0.1 ns printed enum-width difference does not prove that width has no cost in large live register files, lists, maps, or decoded code. The stated 0.15 ns difference needs the unrounded data.

The design reaches `src/term.rs`, all modules in `src/term/`, the BEAM loader, operand decoding, the interpreter, process frames, mailbox signals, ETS, persistent terms, binary construction and matching, native term conversion, equality, hashing, ordering, and printing. Each implementation task must search all callers of its changed interface. Fixtures and tests must cover the same paths. Add no crate or configuration without a consumer.

## Decisions

1. Keep immediate numbers and identifiers in the term. Store each compound root in one `Arc`. Use borrowed reads, ownership moves, and root clones at actual ownership boundaries.
2. Keep the current safe Rust enum during dispatch work. Retain 16 bytes as a later layout target on the verified 64-bit host, not as the first required change. Do not use NaN boxing or raw tagged pointers in the first delivery.
3. Keep the existing 16-way map HAMT and flat shared tuples. Compare a bounded collision tree and a 32-way tuple tree only when their measured write consumers need them. The trees are PBeam proposals, not OTP requirements. Keep tuples of at most 32 elements in one leaf if the tuple-tree gate passes.
4. Keep lists as immutable cons cells. An update builds a new prefix and shares the suffix. It does not edit an old list.
5. Treat #456 and #459 as merged foundations. Verify their current behavior and performance at the new baseline. Keep the flat shared binary view of #456. Add a balanced chunk tree only for the measured append path that retains an old binary.
6. Share immutable roots between processes. Mutable process state remains exclusive. A match context has one process owner and shared register aliases.
7. Keep physical storage sharing separate from ERTS logical copy identity and costs. Do not use pointer equality to answer an ERTS identity query across a copy boundary.
8. Use load-time operation IDs, fixed-arity borrowed arguments, a local execution cursor, and a Rust `match` dispatch loop first.
9. Retain iterative release of nested values. Release private garbage in bounded scheduler work when a large release would block another process.
10. Treat every speed estimate below as a hypothesis. Accept each change only after correctness and its stated measurement gate pass.
11. Take a source register only when it is also the destination, all other operand reads are complete, and failure preserves the required register state. Use `Arc::make_mut` on private map paths. A count of one is not permission to change a value that guest code can still observe.
12. Pre-compute label targets, function metadata, imports, BIF IDs, and constants during loading. Keep frame names as IDs and format names on an error path. Do not replace the scheduler process table merely to remove a per-instruction lookup.
13. Keep atom IDs as the later count-free representation. Reject leaked dynamic atom names. Keep the atom-table owner with host results. Do not make an ID valid across runs without import.
14. Keep initialized-slot checks and size X storage from validated execution requirements. Do not require a 1027-term array per process. Measure fixed storage against right-sized storage before changing this choice.
15. Keep `Term: Send + Sync` and `Arc`. Do not reverse merged #459 or adopt `!Send` and `!Sync`. Both `Rc` and `Arc` can release acyclic graphs without a collector; `Arc` also keeps the accepted thread-safe ownership boundary.
16. Use the second design's count-share trigger as an investigation gate: after dispatch work, a loop above 60 ns per original instruction with counts at least 20 percent of loop time requires a new profile and store comparison. It does not authorize a tagged-word store or replace the 10 ns goal.
17. Keep 40 ns per original instruction as an intermediate dispatch target. Keep 10 ns as the first final target, 3 ns as the stretch target, 15 ns as the final walk gate, and 4 ms as the full redraw gate. Do not stop at 60 ns.

## Reference review and storage rules

The owner reference separates Erlang values from OTP storage and from PBeam proposals. Retain that separation. The reference did not verify OTP source pages. This revision checks the pinned source at `OTP-29.1.1`. The annotated tag object is `906667d86dc2a4b1b8de33e39844bba257b0bb76`; its source commit is `ad05823719d77c8faee87348ea39513d4e2f99c5`. Generic tag sketches do not replace that source.

Immutability means that an old observable value does not change. A register slot copy, a container update, and a process send are different operations. Persistent path copying is explicit allocation and sharing. It does not use operating-system page copy-on-write. OTP uses process garbage collection for ordinary terms. PBeam keeps the accepted acyclic `Arc` graph and shares roots across processes.

| Operation | Current baseline or ordinary storage rule | Proposed optimization and evidence gate |
| --- | --- | --- |
| List prepend or prefix update | One prepend creates one cons. An ordinary prefix replacement at position I, counted from one, rebuilds I cells and shares the suffix. Replacing the last element rebuilds N cells. Heads retain nested roots. | Keep cons storage. Measure prefix allocation, count changes, and final release separately. Do not add a new list representation. |
| Tuple update | An ordinary `setelement/3` BIF copies N slots. Compiler tuple-update instructions can use a safe in-place path. Compound elements retain their roots; this is not a deep copy of their contents. | Compare a tree only for retained-version writes. Keep flat reads unless the bounded-copy and read-regression gates pass. |
| Flatmap update | A bounded array update copies slots and retains nested roots. OTP can share a separate keys tuple for an unchanged key set. PBeam currently stores pairs and does not have that keys-tuple representation. | Do not add a keys cache or shared shape without a measured consumer. Test transitions at 32 and 33 keys. |
| HAMT update | Copy the changed path and retain other subtrees. Hashing, exact key comparison, collision work, node width, and count changes all contribute. | Keep PBeam's existing branch width. Pinned OTP also uses 4-bit hash steps. Keep this source fact separate from proposed 32-way tuple or collision trees. |
| Nested map update | Build the changed inner map, then replace its root in a new outer map. Other retained containers still refer to the old inner map. | Test retained inner and outer versions together. A unique outer root does not make its descendants unique. |
| Binary slice | A view retains its backing allocation. A small slice can keep a large buffer alive. Current unaligned slices also allocate packed bytes. | Measure retained backing bytes and final release. Do not add automatic slice compaction without a consumer and observable-behavior checks. |

The [OTP Maps guide](https://www.erlang.org/doc/system/maps.html#how-maps-are-implemented) confirms keys-tuple and HAMT sharing. The [binary guide](https://www.erlang.org/doc/system/binaryhandling.html) warns that its layout description predates the OTP 27 rewrite. Use its ownership concepts, not historical `ProcBin` names as an ABI.

### Pinned source analysis

| Path | Pinned source evidence | Effect on this proposal |
| --- | --- | --- |
| List append | [erl_bif_lists.c:139–164](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bif_lists.c#L139-L164) copies left cells and retains head terms and the right tail. [Lines 61–88](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bif_lists.c#L61-L88) validate `L ++ []` and can return L. | Separate general append from the empty-right fast path. The latter still has traversal, reductions, and traps. Retain improper-right behavior and reject improper left lists. |
| Ordinary tuple update | [bif.c:2976–3001](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/bif.c#L2976-L3001) validates the index, allocates a tuple, and copies slots. | Count slot copies rather than nested payload copies. Preserve old tuples and invalid-index errors. |
| Compiled tuple update | [beam_ssa_destructive_update.erl:75–90](https://github.com/erlang/otp/blob/OTP-29.1.1/lib/compiler/src/beam_ssa_destructive_update.erl#L75-L90) requires a dead source and alias-analysis uniqueness for `inplace`. [emu/instrs.tab:804–833](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/emu/instrs.tab#L804-L833) also checks the heap high-water boundary and copies otherwise. | Compare dynamic BIF, compiled copy, and compiled private update paths before a tuple-tree decision. OTP heap checks are not PBeam `Arc` ownership checks. |
| Flatmap | [erl_map.h:57–73](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.h#L57-L73) separates keys and values. [erl_map.c:2131–2209](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.c#L2131-L2209) reuses keys for replacement and constructs new keys for insertion. [erl_map.h:93–97](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.h#L93-L97) sets the normal limit to 32, or 3 under `DEBUG`. | Distinguish current PBeam pairs from OTP shared keys. Record ERTS build mode for threshold probes. |
| HAMT update and collision | [erl_map.h:197–220](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.h#L197-L220) defines 16-way branches and sorted flat collision tuples. [erl_map.c:2714–2778](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.c#L2714-L2778) copies path nodes and collision slots. | The bounded collision tree is a PBeam alternative. OTP does not give collision writes a fixed 16- or 32-entry bound. |
| Exact equality | [utils.c:1125–1158](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/utils.c#L1125-L1158) skips identical terms. [Lines 1436–1510](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/utils.c#L1436-L1510) compare map child arrays and skip identical corresponding terms. | Shared-root and shared-subtree checks are valid techniques. PBeam still needs a correspondence proof and structural fallback. Different addresses are not proof of inequality. |
| Numeric map comparison | [utils.c:2045–2048](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/utils.c#L2045-L2048) and [lines 2112–2140](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/utils.c#L2112-L2140) use exact keys and exact or numeric values. [Lines 1422–1434](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/utils.c#L1422-L1434) compare float bits for exact equality. | Future numeric tests must distinguish key equality from value equality, including signed zero. |
| Binary slices | [erl_bits.h:167–168](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.h#L167-L168) defines the 64-byte heap limit. [erl_bits.c:2635–2668](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.c#L2635-L2668) copies small slices and shares backing for larger slices. | Do not claim every OTP slice retains the large backing. PBeam physical retention and `binary:referenced_byte_size/1` require separate evidence. |
| Binary append and publication | [erl_bits.c:1757–1771](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.c#L1757-L1771), [lines 1809–1822](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.c#L1809-L1822), and [lines 1851–1921](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.c#L1851-L1921) check writer state and copy on the non-writable path. [copy.c:812–848](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/copy.c#L812-L848) pins published backing; [erl_bits.c:2051–2079](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.c#L2051-L2079) clears writer flags. | Root uniqueness alone is insufficient. Keep publication, writable-state, and failed-construction tests for any append tree. |
| GC and message copy | [erl_gc.h:42–112](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_gc.h#L42-L112) uses forwarding addresses during relocation. [erl_message.c:767–798](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_message.c#L767-L798) selects ordinary or sharing-preserving copy under `SHCOPY_SEND`. | GC preserves aliases. Message-copy sharing depends on the selected path and build configuration. Do not generalize one identity probe to all OTP copies. |

These are source conclusions, not new performance measurements. Small-map no-change paths and compiler folding can avoid work. Inspect the compiled instructions and retained roots before attributing an O(N) update cost to a workload.

An owning term cannot become `Copy` merely by storing a tagged machine word. Each stored compound alias must retain its owner. A raw non-owning word needs a separate lifetime owner. Keep safe `Clone` and `Drop`; any later tagged representation needs complete retain, release, alignment, pointer, and exit-path proofs.

## Complete representation

The following sketch gives the later ownership boundary. It is not the current representation. The existing 40-byte enum stays during the first dispatch tasks. The compiler reports a 32-byte binary descriptor and an 8-byte term discriminant. Box the binary descriptor in the layout comparison; an atom change alone cannot now reach 16 bytes. Measure the added binary-view allocation and indirection.

The sketch gives the ownership boundary. It is not an ABI definition. The private constructors enforce the small-integer range and valid kinds.

```rust
// Target: 16 bytes on the verified 64-bit host.
enum Term {
    Small(i64),
    Float(f64),
    Atom(u32),
    Nil,
    Pid(u64),
    Port(u64),
    Boxed(Arc<Object>),
}
```

`Object` selects bignum, reference, tuple, cons, map, binary view, local fun, external fun, or match context. This enum is not copied on a root clone. It has a private sized payload so `Arc::into_inner` can take its last value. Do not use `Arc::into_inner` on an unsized slice. Assert the actual term and slot sizes in the layout task. A size target does not create a language-level Rust layout guarantee.

| Kind | Storage and ownership | Read and clone rule |
| --- | --- | --- |
| Small integer | `i64` immediate in the range minus 2^59 through 2^59 minus 1 | No allocation or count change. Overflow enters the integer slow path. |
| Bignum | `Arc<Object>` with sign and normalized `Vec<u64>` limbs | Share the root. Arithmetic creates a new result. Remove leading zero limbs. Convert a fitting result to a small integer. |
| Float | Immediate `f64` | No count change. Preserve ERTS arithmetic, equality, ordering, error, and text rules. Do not use Rust default printing as an ERTS formatter. |
| Atom | Immediate program atom index | The runtime atom table owns each immutable name. A returned host value retains its runtime owner outside the term, so a name cannot outlive its table. Static and dynamic atom identity remain distinct from name order. |
| Reference | `Arc<Object>` with the full reference identity | Do not pack the current counter into a narrower immediate. Retain the node identity, creation data, reference numbers, and alias marker that supported reference kinds need. |
| PID and port | Immediate current local identity | Retain all 64 bits of the current ID. A future external identity uses a boxed payload when its consumer is implemented. Do not claim distributed identity support. |
| Tuple | `Arc<Object>` root with arity and a leaf or a 32-way tree of `Arc` nodes | Share the root. A leaf owns up to 32 terms. Internal nodes own up to 32 child handles. A read descends by 5-bit index fields. |
| List | `Arc<Object>` cons with head and tail terms; immediate `Nil` | Share one cell on a clone. The head and tail retain their own compound roots. Proper and improper lists use the same cells. |
| Map | `Arc<Object>` map root; existing flatmap or 16-way HAMT with shared nodes | Share the root. Leaves own key and value terms. Full-hash collisions use an ordered persistent 32-way tree with bounded leaves. Iteration retains the pinned ERTS order. |
| Binary and sub-binary | `Arc<Object>` view with backing root, bit offset, and bit length; backing is a shared flat byte block or append tree | Share the descriptor. A sub-binary makes a descriptor, then shares the backing. Keep the logical origin size separate from chunk capacity. |
| Local fun | `Arc<Object>` with shared immutable code entry and owned free terms | Share the root and all nested roots. Fun equality uses semantic identity and free values, not the current dispatch address. |
| External fun | `Arc<Object>` with module atom ID, function atom ID, and arity | Share the root. Resolve the export separately. Keep MFA identity when code dispatch data changes. |
| Match context | `Arc<Object>` with a shared binary view and atomic position state | Aliases in one process share the position. Use the #459 state rule. No send, ETS, hashing, or general printing may receive this internal value. |

Every compound edge stores the same `Term`. Thus a map value can share a tuple root, a list head can share a map root, and a tuple slot can share a binary view. There is no container-specific deep clone.

An atom ID requires runtime context for name lookup. This is a real public API change. Keep an owned host result with its atom-table owner. Do not keep an `Arc<str>` inside every atom term. Add that owner only in the task that moves atom names out of terms. Host terms from different runs need explicit import into the destination atom table.

The current runtime does not have full bignum or float terms. Their rows specify the final representation. They do not authorize new arithmetic support in a layout PR. Keep unimplemented operations on the clear runtime-error path until their own task has pinned ERTS evidence.

## Writes and their full costs

Let N be the number of elements, K the work to hash a key, E the worst comparison work for two keys, C the number of full-hash collisions, B the binary bit length, A the added bit length, and D the tree depth. The tree costs below describe conditional proposals, not current flat tuple or binary behavior. The costs include allocation, handle copies, and count changes. Releasing a last old root is a separate possible linear cost.

| Operation | Copy and sharing | Worst-case work |
| --- | --- | --- |
| Map put | Copy the flatmap leaf or each HAMT node on the key path. Share other nodes and nested terms. An exclusive builder can edit private nodes. The validated same-register take rule can avoid an extra root clone and preserve this case. | At most 32 flatmap pairs and their comparisons. For a HAMT, O(K + 16D + (32 + E) log(1 + C)). A 64-bit hash has at most 16 four-bit levels. Key hashing and comparison can still be linear in key size. A collision update copies bounded leaves and ancestors, not all C pairs. |
| `setelement/3` | Validate the index and tuple first. Make a new logical tuple. Copy one leaf and the ancestor nodes. Share other nodes and nested roots. | O(32D) handle work and D allocations. Arity is at most 2^24 minus 1, so D is at most 5 including the leaf. A 1000-element tuple has D = 2 and copies at most 64 slots or handles. |
| `set_tuple_element` | Edit a private tuple builder. If the path is shared, copy that path first. Publish only a complete tuple. | O(32D) worst case. Do not infer compiler permission to edit a live old tuple from a reference count alone. |
| List update at position I | Build I new prefix cells. Share the old suffix and the new head value. There is no general BEAM in-place list update. | O(I) allocations and handle copies. Prepend is O(1). General append copies the left spine and shares the right term. With an empty right term, pinned ERTS can validate the left list and retain it without new cells. |
| Binary append | Keep a private chunk builder when ownership permits it. For a published binary, add new chunks and copy only a balanced tree path. Share old chunks. Copy a partial last chunk of at most 4096 bytes when needed. | O(A/8 + 4096 + log(1 + B/32768)) byte or node work, plus balancing. A native interface that requires one flat buffer still needs O((B + A)/8) materialization. Charge and measure that consumer separately. |
| Process send | Clone one immutable root into the signal queue. Keep sender order and receiver signal rules. Self-send uses the same root. | O(1) physical clone and queue work with one count increment for a compound root. Logical copy accounting may need a traversal if cached metadata is absent. |
| ETS insert | Preserve object, key, access, and table-type validation. Retain an immutable object root. Change an object by replacing its root. | O(1) payload retention. Table search, key comparison, table resizing, and batch validation retain their existing costs. Do not claim O(1) for the complete current vector-based table insert. |
| ETS lookup | Build the result list and retain each object root. Assign a new logical copy domain to each copied object. | O(number of returned objects) payload handles plus table search. No recursive payload byte copy. |

Decision 11 adds a narrow take path. Read keys and values before removing a map from its source register, because a pair can refer to that register. Validate types, keys, and all required exact-update keys before mutation. On a guard failure or caught exception, keep or restore the source and all required aliases. Do not use process exit as a general substitute for this proof. With a different destination, keep the source. With retained old versions, copy each shared path as before. A unique root does not prove that its descendants are unique. Test both owned and shared nodes with `Arc::make_mut`.

The second design's owned-map estimate of 100 to 150 ns is not verified. Retain its less-than-300-ns owned-map and less-than-600-ns shared-map targets as experiment gates on the 1000-key probe. A missed gate requires a profile; it does not justify changing errors or copying rules. An owned HAMT put still pays for hashing, comparisons, path access, possible allocation, and old-value release. It is not zero work.

Keep collision keys in the exact comparison order used by [erl_map.c:2597–2624](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.c#L2597). Use a balanced ordered tree with at most 32 entries per node. Copy only the search path and bounded split nodes. Adapt iteration and logical map-size accounting to the ERTS collision-array view. The current collision array remains until this focused task lands.

A bounded leaf copy is allowed. A full 1000-slot tuple copy for one write is rejected. Small tuples retain a flat leaf because their bound is 32 terms. Large tuples pay an extra dependent load for each tree level. This is the main representation risk for read-heavy Terra code.

A 16-byte term reduces a 1000-element flat tuple payload from the verified 40 KB to 16 KB before tree overhead. It reduces 1024 term slots from 40 KB to 16 KB if the slot has no extra discriminant. Measure the actual slot layout. Do not infer cache speed directly from these byte counts.

The append tree uses 4096-byte leaves and balanced binary branches with subtree bit lengths. Keep #456 flat blocks for existing contiguous binaries. Keep a leaf cursor in sequential matching, hashing, comparison, printing, and IO walks. Do not perform a root search for each byte. At a chunk boundary, copy only the small bit window needed by that operation. A 1-bit append must not become a permanent one-bit allocation per step; fill private leaves and balance published chunks. The 4096-byte size is an internal storage choice, not an ERTS binary-size limit.

## ERTS behavior and sharing limits

The source references below were read at the `OTP-29.1.1` tag. Storage may differ. Values, errors, message rules, and supported reduction behavior may not differ.

- Immediate and boxed tags are defined in [erl_term.h:70–105](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_term.h#L70). Atom indices are defined at lines 297–305. Tuple arity is limited to 2^24 minus 1 at lines 318–323. PBeam need not use the same physical tags.
- A register move copies an ERTS word in [instr_common.cpp:516–519](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/jit/arm/instr_common.cpp#L516). A borrowed read in PBeam must do no reference-count operation. A stored alias must retain its root.
- `setelement/3` checks the index and type, raises `badarg`, and copies the tuple in [bif.c:2976–3001](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/bif.c#L2976). The tree changes physical cost only. It must retain the old value and create the same logical result.
- The normal-build flatmap limit is 32 (3 under `DEBUG`) in [erl_map.h:93–97](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.h#L93). Flatmap put starts at [erl_map.c:2105](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_map.c#L2105). HAMT insertion builds the changed path or retains an equal value at lines 2477–2503. Retain existing exact hash, key order, iterator order, and merge reduction rules.
- List append copies the left spine and can retain an improper right tail in [erl_bif_lists.c:39–55](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bif_lists.c#L39). Do not add a proper-list check to the right term.
- An on-heap self-send avoids a copy in [erl_message.c:824–826](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_message.c#L824). Other local sends use the selected copy routine at lines 835–860. `SHCOPY_SEND` selects a sharing-preserving path at lines 767–798; record the build configuration before interpreting storage observations. Copy reductions use the strict size check and capped quotient at lines 881–885. The release divisor is 64 words in [erl_message.h:32–36](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_message.h#L32).
- ETS calculates object sizes and copies elements in [erl_db_util.c:3559–3614](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_db_util.c#L3559). Its copy-out path starts at lines 3729–3741. Shared physical storage must still give independent logical ETS copies.
- Heap binaries have a 64-byte threshold in [erl_bits.h:166–175](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.h#L166). A match context retains binary origin, offset, and length in [erl_bits.c:81–104](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_bits.c#L81). Append checks type and unit at lines 1738–1782. Its non-writable branch checks size, allocates, and copies at lines 1851–1918. Private append can reallocate and charges copy reductions at lines 1963–1974. Tree append must follow the logical writable-state and reduction rules, not its physical chunk count.
- Arithmetic dispatch and `badarith` handling are in [erl_arith.c:358–410](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_arith.c#L358). A small-integer fast path must fall back on range overflow. It must not wrap, saturate, or convert to float.
- ERTS diagnostic integer and float text paths are in [erl_printf_term.c:472–483](https://github.com/erlang/otp/blob/OTP-29.1.1/erts/emulator/beam/erl_printf_term.c#L472) and lines 596–599. User formatting also runs through pinned OTP modules. Do not replace those modules with a Rust formatter.

The first design records that its pinned probe returned `{2008,8008,8008,true,{a,b},{a,c},{error,badarg}}`. The first three fields are the word sizes of `[L,L,L,L]` before send, after cross-process send, and after ETS lookup, where L has 1000 integers. The fourth field is self-send identity. The last fields show the old tuple, its changed value, and the invalid-index exception.

Physical root sharing alone cannot reproduce these storage observations. A cached flattened word count can reproduce the copy-size charge without byte copying. It cannot reproduce logical identity or all heap and GC observations. Do not claim otherwise.

When an identity or heap observation gets a supported consumer, add logical allocation provenance. A send or ETS copy creates a copy domain. An occurrence path within that domain identifies a copied boxed object. On the ordinary copier path, repeated occurrences then have separate logical identities even when their physical root is equal. A sharing-preserving copier retains the corresponding alias identities instead. Record the selected copy path and build configuration; do not apply occurrence splitting to every send. Register aliases within the same logical value retain that identity. Literals and reference-counted binary origins need the exceptions of the pinned copy routine. Provenance belongs to logical roots and borrowed views; it must not add an atomic increment to each scalar read.

That model is a proposed follow-up, not a complete heap simulator. Exact `memory`, heap sizes, GC timing, and GC reductions also need a logical process heap and its collection rules. Keep these operations explicitly unsupported until that gate passes. The accepted #459 task already leaves send copy reductions with their separate owner. This proposal does not silently remove that gap. Logical map-node sharing must also preserve supported `maps:merge/2` reductions; physical pointer equality alone is insufficient after a copy domain changes.

## Equality, hash, and order

Keep exact equality, numeric equality, and term order as separate contracts. A shared immutable root can prove equality. Different roots cannot prove inequality. Equal maps can have different insertion histories or storage forms. A HAMT routing hash covers a key, not the complete map value. A matching hash still needs equality checks.

At `57b6629`, tuple, cons, and fun equality already skip shared roots. Map equality still visits pairs (`src/term.rs`, `impl PartialEq for Term`), although `Map::same_form` can test a shared root (`src/term/map.rs`). The inline equality stack is already implemented. A measured map-equality consumer can first compare the size and shared root. It must retain structural comparison for distinct roots.

Pinned ERTS already skips identical corresponding HAMT children. Shared-subtree comparison in PBeam is a separate candidate after a root comparison has measured results. Check that nodes correspond before skipping a shared child. Do not compare only an assumed update path. Keep a logical key/value fallback for different layouts and collisions. Use an explicit work stack for deep values. Do not add cached content hashes without a measured comparison consumer and a complete invalidation rule.

For supported kinds, acceptance must include equal maps built in different orders, independent equal roots, shared roots, one changed nested value, forced hash collisions, flatmap/HAMT transitions, improper lists, and non-byte-aligned bitstrings when comparison support is added. Keep the current unsupported bitstring-comparison boundary explicit. Measure visited pairs, allocations, count changes, and worst scheduler pause. Shared-root equality must avoid a graph walk. Distinct equal values can still require linear work.

Numeric support remains outside this optimization revision. When its consumer arrives, test `1 =:= 1.0`, `1 == 1.0`, both numeric map keys, integers outside exact float precision, and signed zero on pinned ERTS. Include numeric map comparison: equal signed-zero values compare equal, but signed-zero keys remain distinct. Since OTP 27, strict equality distinguishes positive and negative floating zero, while numeric equality does not. [OTP term comparisons](https://www.erlang.org/doc/system/expressions.html#term-comparisons) define these rules. Rust `f64::PartialEq`, derived container equality, and atom-ID order are not complete substitutes. Do not add floats or bignums just to run this proposal's optimization tests.

## Cross-process ownership and release

Use `Arc` at each compound root and persistent node. Require `Term: Send + Sync` with a compile-time check. Do not use `unsafe impl Send` for a graph that contains `Rc`. Only the process owner changes registers, frames, a private builder, or match state. Publishing a builder removes its mutation access. A send, timer, ETS entry, or persistent term cannot publish a match context.

An uncontended compound clone needs one atomic count increment. A non-final release needs one atomic decrement. A borrowed read needs neither. A scalar copy needs neither. A compound result stored in a destination register usually needs one increment, and replacement releases the old destination. A move that transfers ownership avoids the increment. Do not clear an X register at a call if BEAM can still observe it.

The current temporary probe measured about 10 ns for an uncontended `Arc` clone/drop pair, against about 4.3 ns for `Rc`. This is a pair cost, not 10 ns for each atomic operation. Keep 5–15 ns per operation only as the old unverified planning range; replace it with full-path count measurements before a forecast. At two operations per instruction, that range adds 10–30 ns and defeats the 10 ns target. Therefore the main plan removes generic operand clones and temporary owners. Four compound clone/drop pairs per three-instruction iteration would add about 23 ns relative to the measured `Rc` pairs, or about 7.6 ns per instruction. That is a conditional estimate. The integer loop does not establish four compound pairs; integer copies have no root count. Cross-core contention can be much slower. Test it before claiming multi-scheduler speed.

Normal reference counting releases immutable DAGs without a tracing collector. It can still pause the thread that releases the last large root. The second design's claim that final release costs only one decrement and one free omits the nested graph traversal. Use `Arc::into_inner` on each sized last-owned node, then place its children on the existing iterative work stack. Do not use `strong_count == 1` as a concurrent release proof. Do not use `try_unwrap(...).ok()` for concurrent final release because both contenders can miss ownership and trigger recursive destruction through an ordinary drop.

Dropping the last root of a million-cell list is still O(N). For interpreter-owned releases, keep the private release work list on the scheduler and process it in batches of at most 256 nodes. This batch is a host cleanup bound, not a guest term limit or a new guest reduction charge. The release work owns its child roots. Thus it cannot lose them on a yield. A public host-side term drop remains iterative and synchronous. Measure pause time and retained bytes. Do not claim a strict time bound from the node-count bound.

Exclude cycles from immutable guest storage by construction. Store code entry IDs rather than a strong back-reference from a fun to an owning module that holds the fun. ETS and persistent tables are owners outside the graph. This acyclic construction rule applies to guest values. Process, registry, and resource ownership need separate cycle checks. An object sent to another process can outlive its creator. Test sender exit, receiver retention, ETS retention, and the final cross-thread release. Resource authority does not move merely because a term root is shared. Keep the capability transfer checks of the existing runtime.

## Interpreter changes

| Change | Concrete execution change | Measurement hypothesis | Main correctness risk |
| --- | --- | --- | --- |
| Resolve BIF IDs | Resolve MFA and arity during loading. Select a fixed operation enum. Use borrowed 1-, 2-, or 3-argument paths for pure leaf BIFs. Keep owned arguments for traps and effects. Include arity 4 where a supported BIF needs it. Reject a reusable owned vector as the first pure-leaf path because it still clones arguments and needs rules for re-entry and traps. | Added `+` and `tuple_size` cost falls from about 300 ns to 30–80 ns. | Guard failure, exception class and reason, trap continuation, validation order, and code loading. |
| Local execution cursor | Retain the active process, frame, module ID, IP, register slice, and stack view while a straight block runs. Commit the cursor before calls, traps, signals, yields, and effects. | Bare loop falls from about 510 ns to 150–300 ns per iteration when combined with BIF IDs. | Stale state on a yield or exception; changed signal switch points. |
| Compact terms, after dispatch evidence | Compare the 16-byte term and immediate atom IDs. Separate immediate cleanup from compound release. Read fields by reference. Move only when liveness permits it. | Full Terra gains 1.2–2 times if register traffic and root counts matter. | Cross-run atom identity, last-owner release, nested equality, and ABI callers. |
| Decoded operands | Store X, Y, literal, and destination indices in operation-specific fields. Resolve label indices and function metadata once. Keep names out of each call frame. Validate table entries and preserve error positions. Allocate X slots from verified module requirements at call entry. Keep Y frame tags and initialization checks. | Bare loop target becomes 45–120 ns per iteration, or 15–40 ns per original instruction. | Malformed BEAM, uninitialized slots, catch tags, dynamic call bounds. |
| Short combined instructions | Combine measured test, arithmetic, and tail-call shapes. Keep the original instruction map and exact intermediate error state. Do not cross an ERTS switch point. | Terra target is 5–15 ns per original instruction. | Lost exception position, reduction charge, or signal observation. |

Decision 12 extends the earlier loader plan. Remove both label and function scans before layout work. Cache module and function IDs in frames. Keep names in module metadata for diagnostics. Resolve BIF imports once through the existing BIF definitions. Preserve unknown-import errors and supported effect routing. A local process borrow removes the hot lookup without a new slot table or free list.

Decision 14 keeps right-sized X storage. ERTS allocates 1027 slots for its own execution layout; this does not require PBeam to allocate the same physical array. At the current 40-byte term width, 1027 slots need 41080 payload bytes per process. Even at 16 bytes they need 16432 bytes. Removing `Option` alone saves no slot bytes at current HEAD. A sentinel must retain the difference between an empty slot and guest `Nil`. The second design's `Op { X(u16), Y(u16), Const(u32) }` sketch does not prove a 4-byte enum: a Rust discriminant can make it larger. Measure the actual operand and instruction layout. Treat 4-byte operands and 32-byte instructions as targets, not results.

These estimates overlap. Do not multiply them to claim a final speedup. The bare loop and full Terra measurements decide whether they hold.

Keep a Rust `match` as the first dispatch. Its code generation can already use a jump table. Compare it with a handler table on the same decoded instruction stream. A function-pointer table can lose inlining and add an indirect call. Rust has no stable C-style computed goto. Do not add a C execution loop only to obtain that syntax.

Keep checked slice access first. A validated module can remove repeated semantic validation without unsafe indexing. A later local unchecked accessor needs proof of the register bound, initialized state, frame size, and lifetime at every dynamic entry. It also needs malformed-bytecode tests and sanitizer or Miri evidence. Adopt it only if bounds checks account for at least 10 percent of the remaining loop time and the measured gain is at least 10 percent.

Use checked integer arithmetic directly for the small-small case. Use raw float registers only on BEAM float instructions that provide that contract. Keep term arithmetic on its type and overflow paths. Do not enable fast-math. Test signed zero, mixed integer/float comparison, conversion overflow, and invalid floating results against pinned ERTS before enabling a new numeric path.

## Delivery and acceptance

Each row is one focused PR candidate. Create task issues under the owning epic after design review. Use native blocked-by links. Use merged #456 and #459 as prerequisite evidence. Do not reopen them to replace `Arc`. The dispatch work already has epic #506 and its task issues. Use their current state and measurements instead of recreating rows 1 to 5 as new tasks. New term work needs owner review of this proposal before implementation. Merge each prerequisite before its dependent PR begins. These rows are a design sequence, not a second task tracker.

| Order | Focus and dependency | Acceptance evidence | Progress measurement |
| --- | --- | --- | --- |
| 1 | Baseline probes; no runtime change | Compile and disassemble each measured function. Confirm that every timed iteration consumes its result. Record revision, host, toolchain, scheduler count, and ERTS JIT build. | All seven term probes, both bare-loop lengths, BIF deltas, binary-size slope, Terra frame time, and allocation counts. |
| 2 | Verify merged #456 at the baseline | Existing binary fixtures retain values and exceptions. Test nested clone, sub-binary backing, offsets, and match context backing. | 100 KB call byte-copy count is zero. Call times at 1 KB and 100 KB differ by less than 10 percent after noise. |
| 3 | Verify merged #459 after #456 | `Send + Sync` check, cross-thread final release, deep graph release, no context escape, signal order, ETS isolation, and existing ERTS comparisons pass. | Root send has constant payload work. Record clone/drop atomic cost and single-thread Terra regression. |
| 4 | Loader label and function tables, then frame IDs and BIF ID dispatch as separate tasks | Validate every label and function entry and retain exact error text. Test `+`, `-`, `tuple_size`, and `map_get` through compiled BEAM with success, guard failure, exception, and trap/effect fallback where applicable. | No argument vector allocation for those pure leaf operations. Report their added loop costs. Test the second design's `map_get` increment target below 100 ns and a 60 percent call-only cost reduction as hypotheses, with a new profile on failure. |
| 5 | Local execution cursor after 4 | Test call, return, yield, caught error, receive, and module change with exact state and existing reduction tests. | Report bare-loop time and process/frame access counts per original instruction. |
| 6 | Immediate atoms and compact term after 3 and measured dispatch work in 5 | Test atom names after host results outlive a run, import between runs, static atoms, nested terms, print/order/hash, and actual term/slot sizes. Preserve unsupported numeric paths. | Term size is at most 16 bytes on the verified host. Report root counts, register bytes, and Terra frame time. |
| 7 | Large tuple tree after 6 and a retained-version update consumer | Test arities 0, 1, 32, 33, 1000, and tree boundaries. Separate dynamic `setelement/3`, compiled copy-mode updates, and compiled dead-source updates; record BEAM hints. Retain old and changed versions. Test each invalid index and compiler tuple-write path. | A 1000-slot update copies at most 64 slots or handles. Scalar `element/2` and direct tuple reads regress by at most 10 percent. |
| 7a | Bounded map collision storage after 6 and a collision-write consumer; independent of 7 | Test exact collision iteration, insert, replace, delete, merge, and retained old roots against the current pinned hash and comparison rules. | Collision writes copy O(32 log(1 + C)) handles. Test C up to 1000 with a unit-test collision constructor. |
| 8 | Shared append tree after 2 and 6 and a retained-prefix append consumer | Test repeated retained-prefix append, private append, slices at 64 and 65 bytes, publication that clears writer state, bit alignment, failed construction, matching, hashing, IO, and logical append reductions. | No old payload copy for shared append except one partial 4096-byte leaf. Match scan throughput regresses by at most 10 percent. |
| 9 | Operation-specific operands after 5 | Test invalid operand forms, bad labels, register limits, empty slots, Y catch tags, and dynamic calls. Keep checked access. | Bare loop is at most 120 ns per iteration; report decoded instruction bytes and branches. |
| 10 | Short combined instructions after 9 | Compare fused and unfused execution for values, exception state, stack traces, reduction boundaries, and message order. Test every exit from each combined instruction. | Terra walk is at most 15 ns per original instruction. Full redraw is at most 4 ms. |
| 11 | Bounded scheduler release after 3 | Test shared last release, process exit, shutdown, and million-cell cleanup without stack overflow or lost roots. | Report maximum scheduler pause, release throughput, and retained memory against synchronous release. |

For a measured map-equality consumer, compare the shared root before designing subtree skipping. Keep these as separate focused changes. Neither needs the compact-term layout or a tuple tree. Read current issue ownership before opening a task.

Add a separate same-register map-take task after the baseline and loader tasks. Test source/destination aliasing, key/value aliasing, a different destination, shared descendants, retained old maps, invalid keys, `put_map_exact` failure labels, and caught exceptions. Measure owned and retained-version map puts against the 300 ns and 600 ns experiment targets. Preserve all old roots.

The 16-byte layout row also needs at least a 10 percent end-to-end Terra gain or at least a 20 percent peak-live-byte reduction with no time regression above 10 percent. A narrow clone/drop result alone does not pass that gate. Tree rows remain conditional; they are not prerequisites for compact dispatch or the full redraw measurement.

If a row combines independent mechanisms, split it into native sub-issues before implementation. In particular, numeric support and logical heap observations are separate consumer tasks. They are not hidden inside the compact-term row.

Every runtime PR has focused tests in `tests/`, compiled fixtures where behavior is observable, pinned ERTS results, and `mise run check` with 100 percent application line coverage. Register each new integration or feature test in `Cargo.toml`. Use `mise run test-focus` during the change. Run `mise run lint` for documentation and `mise run check` before a commit or PR. No test disables or coverage exclusions are allowed. The document does not add implementation tests or new dependencies.

## Measurement plan

Use release builds for timing and separate coverage builds for correctness. Use the same compiled probe modules for PBeam and `erl +S 1`. Keep the user loaded-host measurements as historical evidence. Run the acceptance measurements on a quiet host with recorded load, power state, architecture, and compiler options. Alternate ERTS and PBeam run order. Warm each runtime, then collect at least 30 independent samples. Report median, minimum, p95, and a confidence interval for the ratio. Do not treat a one-run minimum as an acceptance gate.

Use sizes 1, 32, 33, 1000, and 100000 where the operation allows them. Use binary sizes 0, 64, 65, 1024, and 102400 bytes. Test tuple reads with immediate and compound results separately. Retain old map, tuple, and binary versions during write probes. Include collision maps and deeply nested values. Test a million-cell list for release and iteration. Measure maximum live bytes as well as time. Separate container-slot copies from nested payload-byte copies. Add list prepend, middle and last replacement, small and large map updates, nested map updates, shared and independent equality, and small binary slices retaining large buffers. Count all retained child handles on copied paths, not only the root clone. Run separate map probes for same-register owned updates and fresh-destination or retained-version shared updates. Verify the actual register operands in the compiled BEAM.

Measure cycles, retired instructions, branch misses, cache misses, allocation count, copied bytes, and root count operations where host tools permit them. Use temporary instrumentation outside the timed production build. Record the tool and its limits. Repeat `Rc` and `Arc` clone/drop pairs with immediates, atom roots, tuple roots, shared HAMT paths, and contended cross-thread release. Count real ownership operations per original instruction. Do not infer them from source moves alone. Inspect release assembly for the scalar loop before making a bounds-check or dispatch claim.

Record time per original BEAM instruction after combining instructions. Otherwise fusion changes the denominator and creates a false gain. Measure the bare loop at 20000 and 200000 iterations to separate startup from the per-iteration slope. Keep a consumed checksum after the timed loop. Verify disassembly so a compiler cannot remove or replace the intended work.

Measure Terra render and patch separately with the same pinned application and terminal size. Use the PTY scenario in `docs/terra-target.md` to compare screens, key behavior, and terminal restore. The frame gate requires the full path, not only an isolated cell loop.

## Rejected alternatives and failure gates

| Alternative | Reason to reject |
| --- | --- |
| R1. `Arc::make_mut` on each full tuple or byte array | One shared write copies the full structure. This violates the brief. Keep it only for bounded leaves and private builders. |
| R2. `Rc` everywhere, including `!Send` and `!Sync` | Reverses merged #459, owner decision R3, and the multi-scheduler storage boundary. Safe `Rc` works on one thread without a collector, but this proposal keeps `Arc` and pays its atomic cost. An unsafe transfer claim remains invalid. |
| R3. Two local/shared term types with promotion at send | Promotion walks a nested graph on its first send and needs cycle, alias, and ownership rules. It adds a second representation before dispatch cost is fixed. |
| R4. One `Arc` for every integer or atom | It adds allocation and atomics to the most frequent operations. |
| R5. One retained shared arena for all terms | A small message can retain a large unrelated allocation region. Release cost and memory isolation become unclear. |
| R6. A process heap with a tracing collector now | It can approach ERTS word-copy cost, but cross-process sharing needs another collector or copying. It replaces the accepted storage model and does not directly remove generic BIF dispatch. |
| R7. NaN boxing as the first layout | A typical NaN payload cannot hold the full 60-bit small-integer domain. It complicates floating values and pointer width without fixing state access. |
| R8. An 8-byte raw tagged pointer now | It needs ownership, provenance, alignment, clone/drop, and target proofs. First fix dispatch with the current enum. Investigate after the count-share trigger or measured memory pressure. Require an expected end-to-end gain above 20 percent before a new design. |
| R9. A rope for every binary immediately | It adds indirection to flat matching and native interfaces. Use flat shared backing until retained-prefix append is the consumer. |
| R10. Rust default map order or float text | It can change guest results. Keep the pinned OTP hash, order, and formatting paths. |
| R11. A dispatch table or unchecked indexing without evidence | Both can add risk without reducing the measured cost. |
| R12. Leaked dynamic atom names to make `Atom: Copy` | ERTS atom lifetime is the emulator lifetime. PBeam can create and drop many runs in one host. A per-run limit does not bound leaked names across runs. Use IDs with an owned table instead. |
| R13. Fixed 1027-term X array without a profile | Adds per-process storage and initialization cost. The current `Option<Term>` is already the same width as `Term`. Keep empty-slot checks. |
| R14. Reusable owned BIF vector as the pure-leaf default | Removes allocation but keeps unnecessary clones. Borrowed fixed-arity arguments are the smaller ownership cost. |
| R15. Flat full-copy tuples as the final answer to retained-version writes | They keep fast reads but copy O(N) handles on a shared write. ERTS physical copying does not require PBeam to keep that cost. Retain flat tuples until the tree consumer and its gate exist. |
| R16. Ignore `Drop` because it is outside dispatch | Register replacement and temporary arguments can call `Drop` in the instruction path. `Vec::new()` itself does not allocate; profile traversal and last-owner work before changing it. |
| R17. JIT or AOT in this design delivery | The current specification excludes them. Their possible speed is not evidence for this interpreter. |

Reject the dispatch plan if the borrowed BIF and cursor changes leave the bare loop above 300 ns per iteration or reduce full Terra time by less than 20 percent. Use a new profile to find the remaining work before adding representation machinery.

Reject the tuple tree if immediate reads regress by more than 10 percent or retained-version updates fail the bounded-copy gate. Try a different leaf width on the same workload. Do not keep a full-copy large tuple as a claimed solution to this brief.

Reject the append tree if retained-prefix append copies old payload bytes, matching regresses by more than 10 percent, or a flat-buffer consumer removes the end-to-end gain. Keep the append problem open rather than claim success from a construction-only benchmark.

Keep `Arc` even when counts exceed 20 percent of frame time or the merged change regresses single-thread Terra by more than 10 percent. These thresholds require ownership-path work and a profile, not an automatic reversal of #459. First remove unnecessary temporary owners. If the loop remains above 60 ns per original instruction and counts consume at least 20 percent of loop time, compare a counted store in a separate design. The store also needs a host-result owner, complete roots, and a thread-safe ownership rule. Below 20 percent, investigate remaining dispatch and state work. At or below 60 ns, continue toward the acceptance targets; do not declare success or start compilation without a scope decision.

Reject combined instructions if they change exception state, reductions, message order, or supported term printing. Reject the OTP-class performance claim if the final walk remains above 15 ns per original instruction. A 15 ns result is still about 17 times the supplied ERTS instruction time. Meeting the 3 ns stretch target requires further evidence and may require a separately approved native execution design.

## Verification of this proposal

The 2026-10-13 local review read the attached reference, current term ownership and equality code, the data structure audit, and both local optimization proposals. It checked the official maps, binary, and comparison guides. It changes proposed storage rules and acceptance evidence only. It does not approve a new runtime representation or claim new performance results. The private `Box`/`Rc` heap proposal in the dispatch plan is removed because it conflicts with the accepted `Arc` direction and introduces graph promotion. The first local pass could not access GitHub. The source review now has task #551 under epic #450 and milestone tracker #38. It aligns the proposed direction of #518 and the measurement gate of #544; implementation still requires owner acceptance.

`mise run lint` passed outside the sandbox after Taplo failed inside it while reading macOS system configuration. The pinned probe `mise run erts-eval '{1 =:= 1.0, 1 == 1.0, 0.0 =:= -0.0, 0.0 == -0.0, map_size(#{1 => integer, 1.0 => float})}.'` returned `{false,true,false,true,2}`. This verifies those numeric rules on ERTS only. It does not establish PBeam numeric support. No new performance measurements were run for this documentation revision.

### Pinned source-review probes

Read each cited range with `mise run otp-src <path> <start> <end>`. The source table records the exact paths and ranges. These reads use the tag pinned in `mise.toml`.

The command `mise run erts-eval 'Big = binary:copy(<<0>>, 102400), S64 = binary:part(Big, 0, 64), S65 = binary:part(Big, 0, 65), {binary:referenced_byte_size(S64), binary:referenced_byte_size(S65), 0.0 =:= -0.0, 0.0 == -0.0, maps:size(#{0.0 => pos, -0.0 => neg, 1 => int, 1.0 => float}), #{1 => 0.0} == #{1 => -0.0}, #{0.0 => value} == #{-0.0 => value}}.'` returned `{64,102400,false,true,4,true,false}`. This confirms the slice-retention boundary and exact-key versus numeric-value comparisons on pinned ERTS.

For the compiler check, save this temporary module as `/tmp/pbeam_otp_tuple_probe.erl`:

```erlang
-module(pbeam_otp_tuple_probe).
-export([private/1, retained/1]).
-record(r, {count=0, value=old}).
private(Values) -> loop(Values, #r{}).
loop([V|Vs], R=#r{count=N}) ->
    loop(Vs, R#r{count=N+1,value=V});
loop([], R) -> R.
retained(R=#r{count=N}) ->
    {R, R#r{count=N+1,value=changed}}.
```

`mise exec -- erlc -o /tmp /tmp/pbeam_otp_tuple_probe.erl` passed. `mise run erts-eval 'begin {beam_file,_,_,_,_,Fs}=beam_disasm:file("/tmp/pbeam_otp_tuple_probe.beam"), [{Name,[I || I <- Is,element(1,I)=:=update_record]} || {function,Name,_,_,Is} <- Fs,Name=:=loop orelse Name=:=retained] end.'` found `inplace` for `loop/2` and `copy` for `retained/1`, both with tuple size 3. `mise run erts-eval --pa /tmp '{pbeam_otp_tuple_probe:private([a,b,c]),pbeam_otp_tuple_probe:retained({r,3,old})}.'` returned `{{r,3,c},{{r,3,old},{r,4,changed}}}`. The hint does not prove every execution uses the overwrite branch; the runtime heap check still applies.

A separate temporary dynamic-BIF probe returned retained old and new tuples, `[1,2|3]` for `[1,2] ++ 3`, `foo` for `[] ++ foo`, and `[1,2]` for `[1,2] ++ []`. These are value checks, not allocation measurements. No PBeam compatibility expansion or tuple-tree performance claim follows from them.

`mise run check-brief` ran the full `mise run check` gate and passed. Application line coverage was `14734/14734`. The check includes the current reference and runtime tests; it does not prove new optimization performance or unsupported numeric behavior.

### Historical verification from 2026-10-12

Read both designs in full. Read project knowledge, patterns, the relevant benchmark lesson, and the current ownership and map-update paths. Read the runtime specification and README for scope. Git log shows #456 merged through #495 and #459 merged through #496. `gh issue view 459 --json state,title,closedAt` returned `CLOSED`. `gh pr view 496 --json mergedAt,mergeCommit` returned merge commit `a6b838cd24de4330202549829def13e576b664d3` and merge time 2026-10-11 14:42:37 UTC. The compile-time `Send + Sync` bound is in `src/term.rs`.

The layout command succeeded with pinned Rust 1.99.0. Its full output is in the temporary file `/tmp/pbeam-v2-sizes.log`. The clone/drop probe compiled with `mise exec -- rustc -O /tmp/pbeam-v2-count.rs -o /tmp/pbeam-v2-count` and ran successfully. It used `black_box` and checked that each final strong count was one. This was a temporary investigation, not a new benchmark task or an acceptance test. No other second-design timings were reproduced. No current Terra timing, contended count timing, release-assembly review, or new pinned ERTS behavior probe was completed. The ERTS references and probe result above are retained evidence from the first design; they are not new verification in this revision.

`mise run lint` passed. It checked Markdown, configuration, shell scripts, Elixir formatting and compilation, Rust formatting, and Clippy. The final targeted command `mise exec -- rumdl check docs/term-performance-design-codex-v2.md` passed after one extra blank line was removed. `mise run check-brief` started before that correction. Its lint stage reported that line and ordered-list errors in the independently created `docs/term-performance-design-deepseek-v2.md`. The latter file was not changed by this task. Its runtime tests also report denied PTY creation. The final result was exit 1, with 818 Rust tests run: 800 passed, 18 failed, and 8 skipped. The Elixir references passed with 100 percent coverage. The failed Rust run did not establish the required Rust coverage result. The full log is `target/check.log`. No test expectation or coverage rule was changed.

## Changes after the second design

The table covers the numbered decisions, the second design's proposals and measurements, and the acceptance and rejection rules. A retained position can gain a stronger check without changing its ownership rule.

| Point | Position in this revision | Evidence or measurement that decides it |
| --- | --- | --- |
| D1: immediate values and `Arc` roots | Keep | Merged #459, owner direction, and specification. Borrowed reads and moves remove unnecessary counts. |
| D2: safe 16-byte enum | Change delivery order; keep later target | Current term is 40 bytes. First fix dispatch. Layout needs the stated time or memory gate. |
| D3: HAMT, tuple tree, collision tree | Keep bounded-write goal; make tree delivery conditional and independent | Flat shared tuples still copy O(N). Retained-version writes and collision probes decide; tuple reads may regress at most 10 percent. |
| D4: immutable cons cells | Keep | Nested sharing works without a second representation. Prefix changes remain O(I). |
| D5: #456 before #459 | Change from planned work to merged foundations | Git history and GitHub merge evidence. Measure current binary-size slope. |
| D6: process ownership and match aliases | Keep | Current `Arc<MatchState>` and escape checks. No guest context publication. |
| D7: physical and logical copy identity | Keep | First-design ERTS probe shows sharing alone cannot reproduce copied word counts and identity. No new heap claim. |
| D8: loader IDs, borrowed BIFs, cursor, Rust `match` | Keep and extend | Second design identifies label and function scans and frame strings. Verify tables and diagnostic text; measure the real path. |
| D9: iterative and bounded release | Keep; correct pause claim | Last-owner release walks nested graphs. A count does not bound the pause to one free. Measure scheduler pauses and retained bytes. |
| D10: hypotheses and correctness gates | Keep | Microbenchmarks are not a runtime profile. No timing permits changed values, errors, reductions, or message order. |
| Current type sizes | Change baseline | Verified current sizes are in the evidence table. Old `b52e297` sizes were not rebuilt. `Term` is 40 bytes, not 32 or 96. |
| `Rc` versus `Arc` costs | Keep `Arc`; add measured cost | Current minimum pairs were 4.258 ns and 9.986 ns. Extra pair cost is 5.728 ns. The second design's exact timings remain attributed. |
| Enum width has no cost | Reject general conclusion | Its clone/drop-only results do not measure cache traffic or peak live memory. The current term increase also increases cons and HAMT leaf sizes. |
| Stop #459 and use `!Send`, `!Sync` | Reject | #459 is merged. Owner direction and the accepted multi-scheduler boundary require thread-safe terms. `Rc` also needs no collector but does not meet that boundary. |
| Same-register take rule | Adopt with stronger failure and alias rules, D11 | Current map path clones its source. Check all pair reads before take and all failure paths before mutation. Measure owned and shared map probes. |
| Loader pre-computation and frame IDs | Strengthen, D12 | Remove both scans and frame name copies. Preserve validation, code entry identity, and exact diagnostics. |
| `Atom: Copy` with leaked names | Reject leak; keep count-free IDs, D13 | A host can run many atom tables. Per-run limits do not bound a process-lifetime leak. Test returned values after run drop and cross-run import. |
| Fixed X array of 1027 terms | Reject unconditional array, D14 | Current payload cost is 41080 bytes per process. `Option<Term>` has no width penalty. Compare time, live bytes, and initialization before a change. |
| Replace scheduler `HashMap` with slots | Reject as a prerequisite | A local process borrow removes repeated lookup. A slot table needs separate consumer evidence and ID-reuse rules. |
| Reusable BIF argument vector | Keep borrowed fixed-arity path | Reuse removes allocation but keeps clones and re-entry state. Preserve owned fallback for effects and traps. |
| 4-byte operands and 32-byte instructions | Adopt as measured targets, not layout claims | A Rust enum discriminant can exceed the sketch's claimed size. Check actual layout and decoded-code cache traffic. |
| Flat tuple as permanent design | Reject for retained-version write goal; keep current flat baseline | ERTS observable results do not require identical physical copying. Read and bounded-copy gates decide the tree. |
| Shared flat binary and writable flag | Keep | #456 is merged. Physical uniqueness alone does not grant ERTS writable status. Retained-prefix append still needs its own consumer and measurements. |
| Shared process sends and ETS roots | Keep | Constant payload retention is separate from queue, table, validation, logical copy, and reduction work. |
| Float, bignum, external identities | Keep future consumer boundary | Do not add unsupported kinds during layout work. Immediate float avoids a mandatory allocation when its feature arrives. Preserve full identity fields. |
| Do not optimize `Drop` | Reject blanket rule | Register replacement calls `Drop`. An empty work vector allocates no buffer. Change only measured release work. |
| Tagged store: above 60 ns and counts at least 20 percent | Adopt investigation trigger, D16 | Profile after dispatch changes. Require complete ownership and API design plus expected gain above 20 percent. No automatic store implementation. |
| Stop at 60 ns or claim 40 ns is final | Reject; use 40 ns as intermediate, D17 | For 280000 instructions, 40 ns alone is 11.2 ms and 60 ns is 16.8 ms. Neither meets a 4 ms redraw. |
| 10 ns first target, 3 ns stretch, 15 ns walk, 4 ms frame | Keep | Same original-instruction denominator and full PTY path. These are acceptance targets, not forecasts. |
| Other micro-costs and summed budget | Do not adopt as verified measurements | No reproduction of allocation, scan, search, word-copy, enum, register, or store probes. A real profile decides attribution. |
| Dispatch and BIF failure gates | Keep earlier gates; add second-design experiment targets | Bare loop at most 300 ns after initial dispatch, then at most 120 ns; full Terra improvement at least 20 percent. Profile missed 60 percent call-cost or 100 ns map-read targets. |
| Map take acceptance | Add experiment targets | Owned 1000-key put below 300 ns; shared put below 600 ns. Include hashing, allocation, release, and alias tests. |
| Tree and unsafe-access gates | Keep | Reads or matching regress at most 10 percent. Unsafe access needs at least 10 percent measured gain and proof of all entry bounds. |
| `Arc` failure gate | Change from ownership rejection to profile trigger | Remove temporary owners first. Keep merged ownership rules until the owner accepts a separate replacement design. |
| Timing method | Keep stronger sampling plan | Quiet host, alternating runs, at least 30 samples, median, minimum, p95, and ratio confidence interval. Nine loaded-host minima are preliminary evidence only. |
| Combined instructions | Keep after measured dispatch work | Preserve original positions, intermediate error state, and supported reduction and signal switch points. |
| JIT or AOT | Keep out of scope | Specification excludes native compilation. The second design's speed estimates do not authorize it. |
| No collector, no arena now | Keep | Acyclic `Arc` graphs need no collector. A store or heap changes host-result ownership and every root boundary. |
| Delivery through focused GitHub tasks | Keep | This document is a proposal. No issue, board, runtime, test, or fixture change is authorized by this revision. |
