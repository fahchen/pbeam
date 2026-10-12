# Interpreter dispatch plan

Owning epic: #506. Step tasks: #508 to #518.

Status: plan for owner review. This document covers the interpreter dispatch work only. The term representation work has its own design documents (`docs/term-performance-design-codex-v2.md`, `docs/term-performance-design-deepseek-v2.md`).

Baseline revision: `a6b838cd24de4330202549829def13e576b664d3` (2026-10-11). All measured numbers in this document come from this revision, unless the text names another source.

Every number below is marked either measured or estimated. A measured number comes from a release build on an Apple M1 Max with 10 CPUs, from the owner's measurements at the revision of the day, or from a run recorded in this document. An estimated number is a forecast. Do not treat an estimate as a result.

## The facts this plan starts from

The current interpreter runs a decoded instruction stream. A `BeamModule` holds `Vec<Instruction>` (`src/beam/module.rs:12-16`), and the loader decodes the whole `Code` chunk into that vector before any process runs (`parse_code`, `src/beam/code/instruction.rs:1441-2129`). Thus the per-visit cost is not a byte-code decode. The per-visit cost is operand access, ownership, table lookups, and String allocation. Section 2 keeps that structure and says why.

The measured floor loop is `test`, `gc_bif '-'`, and `call_only`, run for 20000 and 200000 iterations, as the minimum of nine runs, in a module that holds 20 functions before the loop. At 20000 iterations PBeam takes 10105 us, or 505 ns for each iteration, against 59 us, or 3.0 ns for each iteration, on ERTS. At 200000 iterations PBeam takes 102394 us, or 512 ns for each iteration, against 533 us, or 2.7 ns for each iteration, on ERTS. The ratio is 171 and 192. No PBeam number in this plan is a per-instruction number until the text says so. The floor loop holds three instructions, and it includes one external call. The functions before the loop matter, because each call runs `function_at_label` (`src/runtime/interpreter.rs:2966-2998`), which scans the instruction vector and compares the module name of each `func_info`. Section 8 gives the measured cost of the other two call shapes.

The measured added costs of one instruction on that loop are: `gc_bif '+'` 307 ns, the `bif` of `tuple_size/1` 297 ns, `get_tuple_element` 148 ns, `get_hd` 239 ns, and the `bif` of `map_get/2` on a 1000-key map 633 ns, of which 34 ns is the HAMT walk. A block of nine call and allocate instructions adds 1099 ns. These numbers overlap with the String allocation cost below.

An independent design measured the internal defects of the loop at this revision: two linear scans that cost 80 ns to 25300 ns, two `String` allocations for each call at about 50 ns, an `is_bif` lookup at 76.5 ns, and one argument `Vec` for each BIF at 31.9 ns. It measured one decoded instruction at 112 bytes in that revision and proposed 32 bytes with a 4-byte `Op` and a constant pool. It also proposed `[Term; 1027]` for the X registers at 41080 bytes for each process. At the baseline revision, `size_of::<Instruction>()` is 136 bytes, `size_of::<Operand>()` is 40 bytes, and `size_of::<Term>()` is 40 bytes (measured for this document with the pinned Rust 1.99.0 and the layout log of the second design; the 112-byte number is the design's earlier revision).

For comparison, the measured term cost is: an `Arc` clone and drop of a `Term` 11.2 ns, an `Rc` clone and drop 4.03 ns, and a deep copy of a 1000-element tuple 5102 ns, as the minimum of 27 runs. On the read-and-move path the `Arc` clone is 1.3 percent of one interpreter step. This plan must not spend its budget on term work.

The end-to-end example is the Terra Counter redraw: about 280000 instructions for one frame of 24 by 80 cells, with about 155 ns for each instruction against 0.9 ns on ERTS (measured, end to end; the two numbers come from different paths and neither is a clean per-instruction cost). The redraw is 99.96 percent of one key press. The key latency is 0.09 s to 0.11 s on the release build and 3 ms on ERTS (measured).

ERTS behaviour that every change must keep:

- The X register file of a process holds 1027 slots, because the loader reserves `MAX_REG + 3` (`ERTS_X_REGS_ALLOCATED`, `erts/emulator/beam/erl_vm.h:64`; `MAX_REG` 1024 at line 55). A slot that no instruction wrote is not a term.
- A light BIF of a `call_ext` or a `call_ext_last` costs one reduction, and the switch out happens before the BIF body runs (`bif_instrs.tab:227-246`; JIT `jit/arm/instr_bif.cpp:583-592`). A heavy BIF costs none (`jit/arm/ops.tab:681-688`).
- A BIF that fails with fail label 0 raises its exception, and a BIF that fails with another fail label jumps and raises nothing (`$FAIL($Fail)`, `erts/emulator/beam/emu/bif_instrs.tab:88-94`).
- A trapping BIF continues through its export (`BIF_TRAP`, `erts/emulator/beam/emu/bif_instrs.tab:520-575`). `length/1` is the only guard BIF with a trap (`erts/emulator/beam/emu/bif_instrs.tab:150-190`).
- `map_get/2` has a JIT fast path (`erts/emulator/beam/jit/arm/ops.tab:797-799`), and the HAMT walk charges no reduction (`erts_hashmap_get`, `erts/emulator/beam/erl_map.c:2424-2470`).
- A process starts at `i_apply_only` of its MFA (`erts/emulator/beam/emu/ops.tab:882`; `erts/emulator/beam/erl_process.c:12616`).
- A line number reaches a stack trace through the line table of ERTS. PBeam keeps no line table, and the location of each frame is `[]` (row "Exception stack trace" of `docs/erts-differences.md`). The loader reads a `line` operand and drops it (`src/beam/code/instruction.rs:1999-2002`).

## 1. The dispatch loop

Decision: keep one `match` over `&Instruction`, and add a small cursor that holds the active module, the active frame index, and the instruction index. Do not move to a handler table.

The current loop reads the active frame from the process on each iteration, compares the frame module name with a `String` comparison, fetches the instruction with `Vec::get`, and increments the frame position (`src/runtime/interpreter.rs:593-611`). The module changes only at a call, a return, or an exception. Thus:

- `Cursor { module: ModuleId, frame: u32, instruction: u32 }` lives in a local of `run`. `ModuleId` is a small integer that the code table assigns at load (`src/runtime/interpreter/code_table.rs:23`). A module switch becomes an integer compare.
- A `ModuleId` compare replaces the `String` compare at `src/runtime/interpreter.rs:599`. Refresh the cursor from the process when the frame module differs, when a call enters a frame, when `return_from_frame` pops one (`src/runtime/interpreter.rs:2403`), and when an exception truncates the stack (`src/runtime/interpreter.rs:2593-2600`).
- A borrow of the module code stays alive while the cursor is valid. This is the one place where a raw borrow or a scoped recompute is possible; hold the borrow only across the straight-line block and drop it before any call into the scheduler.
- Keep the process borrow local. The current loop re-borrows `scheduler.current_process_mut()` each iteration (`src/runtime/interpreter.rs:595`). A function call into `self.call_external` needs `&mut Scheduler` and the process at the same time, so keep the pattern of a short process borrow for the instruction body and no long-lived borrow.

Decode once, not per visit: the loader already decodes the whole stream. The plan adds three loader-owned side tables, and no instruction visits them again:

- `jump_targets: Vec<u32>`: the instruction index of each label. `jump` and `label_index` currently scan the instruction vector on each jump (`src/runtime/interpreter.rs:2692-2722`).
- `function_starts: Vec<(label, NameId, arity, start)>`: the function of each entry label. `function_at_label` currently scans the vector on each frame creation (`src/runtime/interpreter.rs:2966-3000`) and `frame` scans it again (`src/runtime/interpreter/code_table.rs:324-342`).
- `max_x: u32` for each function: the highest X register index that the function reads or writes. Step 8 (task #516) adds this table with the X register array, and that step is its first consumer. Do not add the table before step 8.

Common instruction pairs. The loader already joins two shapes: a `call_ext_last` becomes `CallExternal`, `Deallocate`, `Return` (`src/beam/code/instruction.rs:2025-2043`), and a fun call plus `deallocate` plus `return` becomes one `CallFun` with `tail` (`src/beam/code/instruction.rs:388-401`). Keep those. Add one pair only with evidence: the measured `test` plus `gc_bif` guard shape, and the measured `gc_bif` plus `call_only` shape. A pair that spans an ERTS switch point is not allowed. A pair that changes the reduction charge is not allowed.

Reductions and switch-out. The current loop charges a reduction in `TestYield` and in `Return` (`src/runtime/interpreter.rs:1607-1618`) and one reduction for each light BIF before its body (`light_bif_switch`, `src/runtime/interpreter.rs:2445-2453`). Keep the charges and their order exactly:

- Hold `fcalls` in a local `i32` for the straight-line block. Commit it to `ProcessState::fcalls` (`src/runtime/process.rs:257`) before each exit from the block, and at the block end. `Scheduler::schedule_next` sets the budget to `CONTEXT_REDS` (`src/runtime/scheduler.rs:42`, `:537`).
- A switch out at the budget must leave the process at the same position as today. `light_bif_switch` subtracts one from `next_instruction` before the yield so that the instruction runs again (`src/runtime/interpreter.rs:2450`). Keep that rule in the cursor: commit `cursor.instruction - 1` for that case, not `cursor.instruction`.
- A trap that saved its state commits the saved position. The `length` guard trap decrements `next_instruction` (`src/runtime/interpreter.rs:829`) and the state lives in `ProcessState::length_trap` (`src/runtime/process.rs:279`).
- A combined instruction must charge the same reductions at the same points as its two parts, and must re-run correctly after a switch out. The yield check happens before the BIF body, as `call_bif` does at `erts/emulator/beam/emu/bif_instrs.tab:227-246`.

Acceptance for this section: a compiled fixture drives a `receive` with a timeout across a reduction switch out, and drives a signal that arrives between two turns. The values, the message order, and the reduction totals match the pinned ERTS run.

## 2. Instruction representation

Decision: keep the current decoded form for steps 1 to 6. Add a second, prepared form at load time only in step 8, and only if step 6 measures a cache gain. Do not replace the decoded form in the same step that adds the cursor.

The size argument (measured at the baseline revision):

- `Instruction` is 136 bytes. The largest arm is `BinaryGetBinary` at 132 bytes; `Bif` is 124 bytes with two `String` fields and one `Vec`; `PutList`, `GetList`, and `MakeFun` hold two or three 40-byte `Operand` values.
- `Operand` is 40 bytes. Its `Value(Term)` arm holds the whole 40-byte term.
- Terra compiles to about 280000 instructions for one redraw frame. At 136 bytes this stream is about 38 MB (estimated from the measured instruction count and the measured size). The hot part of the stream does not fit in cache.

The prepared form (estimated): one 32-byte instruction with a 4-byte `Op { X(u16), Y(u16), Const(u32) }` and a module constant pool. Every literal operand moves to the pool. `Bif` becomes `{ fail_label: u32, bif: BifId, arguments: [Op; 4], destination: Op }` at about 26 bytes. `SelectValue` and `TestTaggedTuple` name a pool constant instead of holding a `Term` (`src/beam/code/instruction.rs:611-615`, and the `TestTaggedTuple` variant). The same stream then takes about 9 MB (estimated). This is a target, not a result. The rustc layout of `Op` must be measured, because a Rust discriminant can push the enum above 4 bytes.

The load cost (estimated): one extra pass over the stream and one pool build for each module. The current loader already reads every operand and builds the literal table (`src/beam/code.rs:352-377`). The prepared pass reuses that walk. Do not add a second copy of the stream that lives at the same time as the first; build the prepared stream and drop the decoded one, or keep one representation per module and choose it at load.

The risk to ERTS-exact diagnostics. A line number, an error term, and a stack trace must keep the exact ERTS shape:

- Error terms, exception classes, and reasons come from the BIF body and the instruction arm, not from the encoding. A compact instruction can keep all of them.
- Line numbers are already lost at the loader (`src/beam/code/instruction.rs:1999-2002`). Thus compaction loses nothing that exists today. It also blocks the later line-table work in the backlog entry "Give caller frames and line locations in stack traces". Keep the line index in the prepared form, or keep the raw `line` operand in a side table, so that work stays possible.
- Stack traces are built from frames (`src/runtime/interpreter.rs:3758-3790`). A frame keeps the module and the function name for the trace. The prepared form must keep both, through a name ID and the module name table.
- A malformed module must keep its current runtime error text. The loader tests assert the current text. Keep every check that produces one.

Gate for step 8: measure the instructions retired, the cache misses on the hot loop, and the bytes per instruction with a hardware counter, or with an equivalent measured proxy. Adopt the prepared form only if the compact stream gives at least a 10 percent gain on the floor loop and on the Terra redraw, and no test of the error terms changes.

Step 6 result, measured at `ea4dd13` with the pinned Rust 1.99.0 on an Apple M1 Max with 10 CPUs (task #514): step 8 is skipped and step 9 comes next. This is a decision under incomplete evidence, and it rests on the gate of step 8. `size_of::<Instruction>()` is 136 bytes and `size_of::<Operand>()` is 40 bytes, as the baseline revision records. One iteration of the `call_only` floor loop retires 4 instructions and reads 544 bytes of instruction slots, in an 816-byte slot range. The slots are data, because the run loop reads them as memory, so the L1 data cache of the host is the level that matters: 131072 bytes for the performance cores and 65536 bytes for the efficiency cores. The range is thus below 1.3 percent of the smaller L1 data cache, and the loop is taken to be cache-resident after its first iterations. That residency is an assumption, from the two measured sizes and from the flat per-iteration cost at 20000 and 200000 iterations; the host has no hardware counter to confirm it. A memory-system proxy of the host gives a difference between a 136-byte record and a 32-byte record of at most 0.06 ns for each record at or below a 136000-byte stream, with a measured -0.01 ns at the 816-byte working set of the floor loop, and 2.66 ns at the estimated 40800000-byte Terra stream. One floor-loop iteration reads 4 records, so the largest difference is 10.66 ns for each iteration, which is 6.4 percent of the measured 167.5 ns. One added instruction on the same loop costs 77.6 ns to 154.0 ns. Thus the floor-loop gate fails, because that loop is cache-resident, and the estimate for a large instruction stream is 6.4 percent under the proxy, below the 10 percent gate. A later measurement with hardware counters would change the decision if it showed L1 data misses on the instruction slots of the floor loop, or more than 10 percent of a Terra redraw frame in the fetch of the instruction stream; PBeam has no Terra benchmark today, and the host has no `perf`.

## 3. Operand and register access

Decision: read an operand through one bounds-checked lookup that returns a reference, clone only where ownership demands it, and write by value with no clone.

Current state:

- `read` clones (`src/runtime/interpreter.rs:2725-2727`), `read_ref` borrows (`:2740`), `read_any` clones (`:2757`), and `read_any_ref` borrows (`:2771`). All four call `read_operand` (`:3801-3831`).
- `read_operand` checks the X bound, the "is set" state, the Y bound, the Y "is set" state, and the catch-tag state on every access (`:3801-3831`).
- `write_operand` grows the register vector on every X write with `registers.resize(registers.len().max(index + 1), None)` (`:3849-3858`). The register file is a `Vec<Option<Term>>` (`src/runtime/process.rs:185`).

Changes:

- Preallocate the register file for the function before its first instruction. The loader records the highest X index each function touches (`max_x`, step 8, task #516). `Cargo`-level cost: one `Vec` size for each process, grown at call entry to the maximum of its frames, never shrunk. The per-write `resize` goes away (measured removal of a branch and a possible reallocation on each write; the current per-operation cost of the growth path is not measured).
- Store the "no value" state as one sentinel `Term` instead of `Option<Term>`. ERTS uses the tagged word `THE_NON_VALUE`, which no guest term can hold. Add a private `Term::EMPTY` or a private newtype around `Term`, and keep the guards in `read_operand` and `write_operand`. The X array then holds terms with no `Option` tag work, and at 40 bytes per term, 1027 slots need 41080 bytes for each process (measured size, estimated array).
- A checked read that cannot fail after loader validation: the loader already validates each register operand against the declared X limit and each `allocate` size against the Y use. Keep the checked access first, and keep the error text. A later unchecked accessor needs all of: the register bound, the initialized state, the frame size, and the lifetime, proved at each dynamic entry. Adopt it only if bounds checks are at least 10 percent of the remaining loop time and the measured gain is at least 10 percent (the same gate as `docs/term-performance-design-codex-v2.md:196`).
- A borrow into registers and the frame stack at the same time is the hard part. The frame stack lives in `process.frames.last().stack` (`src/runtime/interpreter.rs:2793`), and the registers live in `process.registers`. A read of one of them borrows the whole process. A read that returns a reference must not be held across a write. Keep a single operand read for each instruction argument, and copy to the destination at the end.

Places that must still clone for ownership (name each one):

- `call_arguments` copies each argument into a `Vec<Term>` for a BIF call (`src/runtime/interpreter.rs:3010-3021`). Section 4 moves the pure leaf BIFs off this path.
- A BIF body that takes `Vec<Term>` and keeps it: `BifResult::TrapBif` stores the argument vector in `PendingBif` (`src/runtime/process.rs:321-331`), and the `length` trap keeps it in `length_trap` (`src/runtime/process.rs:279`). These keep the owned fallback.
- A map write and a tuple write that hits a shared form: `Map::get_mut` and the map write operations copy the shared form (`src/term/map.rs:315`), and `Tuple::arc_mut` does the same for a tuple (`src/term/tuple.rs:49`). This copy is guest-visible as an ERTS-style copy and must stay.
- `make_fun` copies each free variable (`src/runtime/interpreter.rs:755-757`).
- A term that crosses a process boundary: a send, an ETS entry, or a persistent term keeps the `Arc` clone of #459 (`src/term.rs:262-295`). Section 5 keeps it.

A write takes ownership. `write` and `write_operand` already take `Term` by value (`:2898`, `:3841`). Keep that. The cost is the drop of the replaced destination value, not a clone. That drop is in the instruction path, and section 6 owns it.

Acceptance for this section: a compiled fixture writes x1023 and reads it; a malformed module that names x1024 keeps its current error text; a Y read of an empty slot, of a catch tag, and of an out-of-range index keeps its current error text. Register the test in `Cargo.toml` as `tests/integration/`.

## 4. BIF calls

Decision: resolve each BIF to a small identifier at load time, pass borrowed fixed-arity arguments for the pure leaf BIFs, and keep the owned argument vector for the BIFs that trap or take ownership.

Current state:

- `is_bif(module, name, arity)` binary-searches a 553-entry table (`src/runtime/interpreter/bif.rs:727-730`; the table is `src/runtime/interpreter/bif/table.rs`, 553 entries) and then tests a 5-entry table. The measured cost is 76.5 ns for each call. It runs on each `Instruction::CallExternal` (`src/runtime/interpreter.rs:1693`), on each `call_mfa` (`:1752`), and on each `Instruction::Bif` (`:1700`).
- `call_arguments` builds one `Vec<Term>` for each BIF call (`:3010-3021`). The measured cost is 31.9 ns for each BIF.
- `table::dispatch` matches `(module, name, arguments.len(), arguments)` with a `&str` compare on each arm (`src/runtime/interpreter/bif.rs:362-379`).
- Name allocation: `Frame` holds `String` for the module and the function (`src/runtime/process.rs:46-55`), and `frame` allocates both (`src/runtime/interpreter/code_table.rs:337-338`). `function_at_label` also allocates the function name (`src/runtime/interpreter.rs:2966-2996`), and `bif_trap` allocates the caller module on the `load_nif` path (`:1687`). This is the two-String-per-call cost, about 50 ns (measured for the pair).

Changes:

- A `BifId` enum with one variant for each leaf operation the table implements at fixed arity. The loader resolves the MFA of an import once, at parse time, and stores `BifId` in the instruction. Choose a total order that keeps the current `BIFS` order, so the identifier is stable.
- `Instruction::Bif` holds `bif: BifId`, no module name and no function name. `Instruction::CallExternal` holds an `ImportId` (already the import index) plus an `Option<BifId>` classification. `CallExternal` of a BIF then reaches the BIF body with no binary search. A heavy BIF keeps its current path.
- `is_bif` remains for dynamic paths only: `erlang:apply/2,3`, `call_mfa` of a name that the code table does not export, and `erlang:function_exported/3`. Those paths keep the measured 76.5 ns, because no load-time fact exists for them.
- Borrowed fixed-arity arguments: `bif` takes `&[Term]` for the pure leaf BIFs. `call_arguments` then copies nothing. The interpreter builds a small array of borrows from the registers, or passes a borrowed slice of a register view. Arity 0 to 3 covers the measured set (`+`, `-`, `tuple_size`, `get_tuple_element` in the `bif` form, `map_get`). Arity 4 stays on the owned path.
- A reusable owned vector is rejected as the pure-leaf default, because it still clones each argument (see section 9).
- Keep the owned `Vec<Term>` path for: a BIF that traps (`BifResult::Trap` and `TrapBif`, `src/runtime/interpreter.rs:2143-2185`), a BIF that takes ownership of an argument, an arity above the borrowed limit, and the `pbeam_cap` bodies (`src/runtime/interpreter/bif.rs:409-426`).
- Keep every validation step of each BIF body: the argument pattern, the guards, the `badarg` order, and the error text. The measured set is `erlang:'+'/2`, `erlang:'-'/2`, `erlang:tuple_size/1`, `erlang:map_get/2`, `erlang:get_tuple_element` (the `bif` form of `element/2`), and `erlang:get_hd` (`hd/1`).

Expected removals (estimated from the measured components, and the numbers overlap): the argument `Vec` at 31.9 ns, the name lookup at 76.5 ns, and the two String allocations at about 50 ns. Do not add them and claim 158 ns. Measure the floor loop before and after each step.

Acceptance for this section: a compiled fixture calls `+`, `-`, `tuple_size`, `map_get`, and `hd` with a success, a guard failure, a caught exception, and a trap where the BIF has one. Each result, class, reason, and reduction count matches the pinned ERTS run.

## 5. The three ownership classes

Decision: keep the owner's split, and move the process-private data from `Arc` to `Box` and `Rc` in step 10.

The split, with its justification:

- Process heap: `Box`, owned by the current scheduler. A term that one process owns and no other process can see needs no shared count. The process heap is the term storage of the process: tuples, lists, maps, and funs that only this process holds. `Box<[Term]>`, `Box<Cons>`, and `Box<Node>` replace the `Arc` forms for that data. The scheduler owns one process heap at a time, and it frees the heap when the process exits.
- Shared binary: `Arc<[u8]>`. The bytes are immutable and shared across processes, so a slice with one shared count is right. The current code uses `Arc<Vec<u8>>` (`src/term/binary.rs:48`, `:58`). Move to `Arc<[u8]>` in the same step, because a `Vec` keeps a capacity field and a separate allocation header that the bytes never change.
- Shared module code: `Arc<Module>`. Loaded code is immutable and shared by every process. The code table holds `Rc<BeamModule>` today (`src/runtime/interpreter/code_table.rs:23`), which is not `Send`, and the runtime holds it behind the scheduler. Move the module code to `Arc` when the code table crosses a scheduler thread, and keep the load path single-threaded.
- Scheduler-local temporary data: `Rc` or a plain reference. A value that never leaves one scheduler uses the non-atomic count or a borrow. `MatchState` is `Arc<MatchState>` today (`src/term.rs:91`) and stays per process; it moves to `Rc` when the process heap is private.

What moves, in order:

- `Tuple(Option<Arc<Vec<Term>>>)` (`src/term/tuple.rs:24`) to a private form with `Box<[Term]>` and a shared form only for a published term.
- `ConsRef(Option<Arc<Cons>>)` (`src/term/list.rs:34`) to the same split.
- `Map { repr: Arc<Repr> }` (`src/term/map.rs:110`) to the same split. This is the largest change, because the HAMT nodes are shared between versions today.
- `Fun(Option<Arc<FunThing>>)` (`src/term/fun.rs:248`) to the same split.
- `Binary` keeps `Arc` and changes `Vec<u8>` to `[u8]`.
- `MatchContext` moves to `Rc` with the process heap.

How a message crosses processes. Today `Term::clone` shares the storage of a compound term and takes O(1) time (`src/term.rs:262-295`), and #459 made `Term` `Send + Sync` with a compile-time bound (`src/term.rs:94-104`). Keep that rule for a published term: a send, an ETS entry, a persistent term, a timer, and a process exit reason keep the current shared form. A process-private term that becomes shared has to be promoted at that point. Promotion walks the term graph and copies the private nodes into shared nodes, or moves the whole heap into a shared form when the last owner sends it. Name the promotion cost at the send path and measure it on a message with a deep term. Do not add a per-instruction promotion check.

How the process heap is allocated and freed. Allocate one heap for each process at its creation, or at its first term allocation, and free it at its exit. A BEAM `allocate` does not allocate the term storage of a process, because the process heap is not a stack. Keep the Y register stack separate, as today (`src/runtime/process.rs:54`). The scheduler owns the heap while the process runs, so no count operation is needed on the per-process path.

The measured atomic cost that this change removes, and where it is paid. An uncontended `Arc` clone and drop of a `Term` is 11.2 ns, and an `Rc` clone and drop is 4.03 ns (measured, minimum of 27 runs). The 7.2 ns difference is the cost of the two atomic operations of the pair. On the current read-and-move path the `Arc` clone is 1.3 percent of one interpreter step (measured). Thus the atomic cost is not the dominant term today. It is paid on each compound clone: `read`, `read_any`, `call_arguments`, `Term::clone` in send and ETS, and each register write that drops a shared destination.

Interaction with task #459. #459 is merged (`a6b838cd`, PR #496) and made terms `Send` and `Sync` and shared them between processes. Keep that decision for shared data, and do not reverse it. The private heap is a second class beside the shared class, not a replacement. A term that no other process saw is private by construction, and the runtime may not publish a private term without promotion. The compile-time `Send + Sync` bound stays.

Gate for step 10: the floor loop, the Terra key latency, and a message-passing workload with a deep term. Adopt the private heap only if the floor loop gains at least 15 percent over step 9 and no send, ETS, and persistent-term value changes. If the counts stay below 20 percent of loop time, keep `Arc` and report the ceiling instead (the rule in `docs/term-performance-design-codex-v2.md:267`).

## 6. The small defects with measured costs

Each defect gets its own change, its own test, and its own measurement gate. Do not bundle them into one PR.

**The drop of a shared `Map` allocates a fresh handle.** `Map::drain_into` replaces the form with `Arc::new(Repr::Flat(Vec::new()))` before `Arc::into_inner` (`src/term/map.rs:503`). The comment at `:504-505` explains why the code cannot use `Arc::get_mut`. The measured cost of dropping a shared map is about 21 ns more than dropping a tuple (measured). Change: make `repr` an `Option<Arc<Repr>>`, take the form with `Option::take`, and hand it to `Arc::into_inner` with no allocation. Update each accessor to read through the option. Gate: the drop cost of a one-key shared map is at or below the tuple drop cost plus 3 ns, and the map operation set (`get`, `put`, `put_map_exact`, iteration, `map_next/3`) shows no regression above 2 percent.

**`Term::Drop` builds a work vector even for an immediate.** `Drop for Term` creates a `Vec::new()` and walks it (`src/term.rs:253-259`), and `take_nested` pushes the nested terms of a tuple, a list, a fun, or a map (`src/term.rs:203-215`). `Vec::new` does not allocate, so the cost is the call, the match, and the loop for a term that has no nested term. Change: a fixed array of 16 pending terms as the first stage, and the heap vector only after the array is full. Keep the iterative walk for the heap stage, because the current code exists to avoid a deep Rust recursion and to avoid a stack overflow on a deep term (`src/term.rs:243-252`). Gate: the drop of an immediate and of a 1-element tuple costs no more than the current immediate drop plus 1 ns, and the drop of a 1000000-cell list does not overflow the stack.

**The per-call String allocations.** `frame` clones the module name and allocates the function name on each frame creation (`src/runtime/interpreter/code_table.rs:337-338`), and `function_at_label` allocates the function name before that (`src/runtime/interpreter.rs:2966-2996`). `CodeTable::entry_label` and `export_frame` also allocate a key for each lookup (`src/runtime/interpreter/code_table.rs:114`, `:149`). The measured pair cost is about 50 ns for each call. Change: replace `Frame.module: String` and `Frame.function: String` with a `ModuleId` and a `NameId`, resolve the names from the loader tables when a diagnostic needs them, and key `exports` by `NameId`. Keep the diagnostic text identical: "function {module}:{function} did not return" (`src/runtime/interpreter.rs:605-608`, `:1620-1626`) and the `function clause` text (`:1630-1635`). Gate: a call-heavy fixture shows no `String` allocation on the frame path, the measured call block of nine instructions falls, and each error text is unchanged.

## 7. The order of work

Each step is one focused PR with one acceptance test, one measurement, and one gate. The step numbers are the PR order. Create task issues under the owning epic after design review, and use native blocked-by links between them (`AGENTS.md`).

| Step | Focus | Acceptance test | Measurement | Gate that unlocks the next step |
| --- | --- | --- | --- | --- |
| 0 | Measurement only, no behaviour change | The benchmark target builds and runs the floor loop at 20000 and 200000 iterations, the BIF deltas, and the Terra redraw. The probe consumes each result, and the disassembly of the loop is recorded. | Record revision, host, toolchain, scheduler count, and the ERTS build. Record the current per-step numbers. | All later steps need these numbers. This step changes no runtime code. |
| 1 | Loader tables: `jump_targets` and `function_starts`. `max_x` moves to step 8, the first consumer of that table. | Malformed modules with a bad label, a bad entry, and a bad register index keep their current error text. | Label and function lookups per original instruction; the measured 80 ns to 25300 ns scan cost. | A jump, a call, and a return cost one table read. |
| 2 | Execution cursor and a local `fcalls` | Call, return, yield, caught error, receive timeout, and module change keep their values, reduction totals, and message order against pinned ERTS. | Floor loop at both lengths. | The cursor is in place before any representation change. The gate is a measured floor loop below 400 ns for each iteration of the quoted shape, the floor loop of a module with functions before it, which the benchmark measures at 505 ns to 512 ns. The gate does not use the local `call_only` shape, which is already at 340 ns in the benchmark. If the loop does not move, stop and profile. |
| 3 | Name IDs in frames and export keys | Each call-path error text is unchanged. A module and a function name with a non-ASCII byte still prints. | The measured two-String cost of about 50 ns; allocations on the call path. | The call path allocates no `String`. |
| 4 | `BifId` at load time, `Instruction::Bif` with no names | Success, guard failure, exception, and trap for the small measured set, against pinned ERTS. | The measured `is_bif` cost of 76.5 ns; the BIF deltas. | One BIF call does one table lookup at load time and none at run time. The gate is a measured BIF delta below 200 ns. |
| 5 | Borrowed fixed-arity BIF arguments | The same BIF set, plus an argument that aliases the destination and a BIF that traps. | The measured argument `Vec` cost of 31.9 ns. | The pure leaf path allocates no `Vec`. The gate is a measured `+` delta below 80 ns. |
| 6 | Keep the decoded form; measure it | No behaviour change. | Instruction bytes for the hot loop; retired instructions; cache misses; the same loop with and without the cursor. | A measured per-instruction cost for the decoded form. If the instruction bytes are not a measured part of the loop cost, skip step 8 and go to step 9. |
| 7 | `Term::Drop` and `Map` drop | A deep term, a shared map, a cloned map, and a 1000000-cell list drop without a stack overflow and with the same values afterwards. | Drop cost of an immediate, a tuple, a shared map, and a deep list. | No measured regression on a term drop, and no map operation regression above 2 percent. |
| 8 | Prepared instruction form with a constant pool, and the X register array (`max_x`) | Register limit, empty slot, Y catch tag, dynamic call bounds, and every current loader error text. | `size_of::<Instruction>()` and `size_of::<Op>()` with the pinned rustc; floor loop; Terra redraw. | At most 32 bytes for each instruction and at least a 10 percent floor-loop gain. If either fails, revert the step and go to step 9. |
| 9 | Combined instruction pairs | Every exit from each pair, its exception state, its reduction charge, and its message order against pinned ERTS. | Floor loop per original instruction; Terra redraw. | The floor loop is at most 150 ns for each iteration. |
| 10 | Process heap with `Box` and `Rc`; binary `Arc<[u8]>`; shared module `Arc` | Send, ETS, persistent term, timer, and exit reason values; cross-thread release; a deep send. | Arc and Rc clone/drop counts for each original instruction; floor loop; Terra key latency. | At least a 15 percent floor-loop gain over step 9 with no value change. If the counts stay below 20 percent of loop time, stop and report the ceiling. |

The order matters in these places. Step 1 must come before step 2, because the cursor needs the tables. Step 4 must come before step 5, because the borrowed path needs the resolved identifier. Step 6 decides whether step 8 is worth its risk. Step 7 is independent of the instruction representation. Step 10 must come after step 9, because a measured dispatch cost decides whether the ownership work can pay.

## 8. Targets and falsifiers

Targets, all as forecasts from the measured baseline:

- Floor loop: 512 ns for each iteration at the baseline (measured). The shape of that number is the floor loop of a module that holds 20 functions before the loop, and the benchmark target `tests/integration/dispatch_baseline.rs` measures it in `fixtures/dispatch/dispatch_position_probe.erl`. The same target measures the local `call_only` shape at 340 ns and the module-qualified `call_ext_only` shape at 564 ns. The target after step 5 is at most 300 ns, after step 9 at most 150 ns, and the stretch target is 34 ns, which is the ERTS 2.7 ns for each iteration multiplied by 12.5. The `docs/term-performance-design-codex-v2.md:261` gate rejects the plan if the borrowed BIF and cursor work leaves the loop above 300 ns.
- Each added instruction: at most 80 ns for each added `gc_bif` after step 5, against the measured 307 ns for `+` and 297 ns for `tuple_size`. At most 40 ns for each added element read after step 8, against the measured 148 ns for `get_tuple_element` and 239 ns for `get_hd`. At most 200 ns for the `map_get` call, against the measured 633 ns, of which 34 ns is the HAMT walk. These are estimates of the achievable result, not results.
- Terra key latency: 0.09 s to 0.11 s at the baseline (measured). The target after step 5 is at most 30 ms, after step 9 at most 10 ms, and the final target is at most 5 ms. The measured ERTS value is 3 ms. The redraw target of 4 ms in `docs/term-performance-design-codex-v2.md:216` needs about 14 ns for each original instruction over 280000 instructions (estimated from the measured instruction count).
- The final floor-loop target is at most 45 ns for each iteration, the lower end of the `docs/term-performance-design-codex-v2.md:185` range for operation-specific operands.

The measurements that would show the plan is wrong:

- The floor loop stays above 300 ns for each iteration after steps 1 to 5. Then the cost is not in the dispatch paths this plan names. Profile again before any representation work.
- The cursor and the BIF identifier give less than a 20 percent floor-loop reduction, or less than a 10 percent Terra reduction. Then the loop is bound by something this plan does not own, and the plan stops.
- The measured `Arc` clone and drop count on the hot path is below 5 percent of loop time after step 9. Then step 10 cannot pay and the plan keeps `Arc` and stops.
- The prepared form does not reach 32 bytes with the pinned rustc, or it changes one error term, one exception class, or one stack trace. Then the plan keeps the decoded form.
- A combined instruction changes a reduction charge, a signal observation, or a message order. Then the plan removes that pair and keeps the two instructions.

The risk that a Rust interpreter has a lower ceiling than the ERTS JIT. ERTS executes one instruction in 0.9 ns to 3.0 ns with hand-written assembly and a register machine. A safe Rust interpreter with a 32-byte instruction, a bounds-checked operand read, and reference-counted terms has a higher floor: an estimated 10 ns to 30 ns for each instruction, which is 3 to 30 times the ERTS number. The plan may reach that ceiling and stop there. The owner then has three decisions: accept the ceiling and the Terra key latency that it gives, approve a separate native execution design, or change the guest workload. Do not start a JIT to hide a dispatch number that the plan did not measure.

## 9. Rejected alternatives

- **NaN boxing.** A NaN payload cannot hold the full 60-bit ERTS small-integer domain, and it complicates the float paths. It also changes term layout, which is not this plan's problem. Reject it here, and let the term representation design decide it.
- **A bytecode-to-threaded-code translator.** Rust has no stable computed goto. A function-pointer table loses inlining and adds an indirect call for each instruction. The current `match` already compiles to a jump table. Keep the `match` first, and compare it with a handler table only on the same decoded stream and with a measurement. Do not add a C execution loop to obtain the syntax (`docs/term-performance-design-codex-v2.md:194`).
- **A garbage collector.** ERTS collects the process heap, and PBeam does not. A tracing collector changes when a term is released, which changes the reductions and the release order that this plan must keep. It also needs the cross-process sharing rules of a second collector. Step 10 uses `Box` and `Rc` with private ownership and no collector.
- **A JIT or AOT.** The specification excludes them (`README.md:24`, `docs/portable-beam-spec.md:46`). Their possible speed is not evidence for this interpreter. Consider them only after the ceiling in section 8, and only as a separately approved design.
- **Unchecked indexing without proof.** Keep the checked access first. Adopt an unchecked accessor only with the proof and the malformed-bytecode evidence of section 3.
- **A reusable owned argument vector as the pure-leaf default.** It removes the allocation but keeps the clones. Borrowed fixed-arity arguments are the smaller ownership cost (`docs/term-performance-design-codex-v2.md:256`).
- **`Rc` everywhere.** It reverses #459 and the multi-scheduler storage boundary. Keep `Arc` at each shared root.
- **One `Arc` for each integer, atom, or pid.** It adds an allocation and an atomic operation to the most frequent operations. Those terms are immediates and stay immediates.
