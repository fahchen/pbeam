# A borrowed-argument interface for a BIF body

Status: design proposal for owner review. Documentation task: [#564](https://github.com/fahchen/pbeam/issues/564). Epic: [#506](https://github.com/fahchen/pbeam/issues/506). Milestone tracker: [#38](https://github.com/fahchen/pbeam/issues/38). The described revision is `9adc310`, the head of `origin/main` at the time of writing. This document changes no runtime behavior.

This document decides one interface: the way a BIF body reads the arguments of a call. Task [#549](https://github.com/fahchen/pbeam/issues/549) measured the BIF call path, and [PR #561](https://github.com/fahchen/pbeam/pull/561) recorded the safe forms that fail. The plan is [the interpreter dispatch plan](interpreter-dispatch-plan.md). The measurement record is [the interpreter dispatch measurements](interpreter-dispatch-measurements.md).

## The measured problem

The parts of one BIF call of `gc_bif '-'` at `4261205`, in nanoseconds for one operation, as the minimum of nine timed runs of 100000 operations ([#549 comment](https://github.com/fahchen/pbeam/issues/549)):

| Part | Base |
| --- | --- |
| `bif_arguments_build` | 24.883 |
| `bif_slice_minus` | 5.900 |
| `bif_result_value_handling` | 7.379 |

The three parts sum to 38.162 ns. The argument buffer build is the largest part at 24.883 ns, which is 65 percent of the sum. The buffer alone costs 6.811 ns when the constructor writes `Term::nil()` into all three slots, against 0.320 ns when it writes no term. `BifArguments::new` writes `std::array::from_fn(|_| Term::nil())` into `terms: [Term; MAX_BORROWED_ARGUMENTS]` (`src/runtime/interpreter/bif.rs:314-335`), and `read_bif_arguments` then overwrites each slot ([PR #561](https://github.com/fahchen/pbeam/pull/561)). Every `bif` instruction of the borrowed path pays 120 bytes of store for at most three filled slots.

The residual cost of the build is the operand read and the `Term` copy of each argument. `read_bif_arguments` calls `read_any` (`src/runtime/interpreter.rs:3165-3167`), and `read_any` clones the borrowed term (`src/runtime/interpreter.rs:2910-2912`). The copy of an immediate term is a 40-byte copy. The copy of a compound term is an `Arc` clone, which the term measurement gives as 10.3 ns for the pair of the clone and the drop (`docs/interpreter-dispatch-measurements.md:229`).

Task [#549](https://github.com/fahchen/pbeam/issues/549) measured every safe form of this interface against the same floor loop, as a paired difference in nanoseconds for each iteration:

| Form of the argument buffer | Paired difference |
| --- | --- |
| `MaybeUninit` slots (unsafe, rejected) | −6.057 to −13.184 |
| `[const { Term::Nil }; 3]`, overwritten | −1.896 to +1.548 |
| `[Option<Term>; 3]` with a copy in `with` | +22.416 to +24.379 |
| `Vec<Term>` with a capacity of three | +0.375 to +8.800 |

Safe Rust has no safe constructor for a partially initialized array, so `[const { Term::Nil }; 3]`, `std::array::from_fn`, `Option`, and `Vec` all initialize or allocate the whole buffer. The best safe form is neutral, and the other two are slower. The `MaybeUninit` form is rejected under the no-unsafe rule of `AGENTS.md`. Thus no form of the current interface reaches the task gate of #549.

## The obstacle

`read_bif_arguments` borrows the process immutably, and `bif_slice` takes the scheduler mutably (`src/runtime/interpreter/bif.rs:695-703`). The process lives in the scheduler: `Scheduler::processes` is a `HashMap<ProcessId, ProcessState>` (`src/runtime/scheduler.rs:155`), and `current_process_mut` returns `&mut ProcessState` from that map (`src/runtime/scheduler.rs:995-1001`). The register file is `registers: Vec<Option<Term>>` of the process (`src/runtime/process.rs:217`), and `read_operand` returns a reference into that file (`src/runtime/interpreter.rs:3973-4005`).

Therefore a slice that borrows the registers also borrows the scheduler. Rust rejects the call `self.bif_slice(scheduler, id, &terms)` while `terms` borrows `scheduler`, because it holds a shared borrow and a mutable borrow of one value at the same time. Step 5 recorded this decision (`docs/interpreter-dispatch-measurements.md:372`), and the current code avoids it by copying each argument into an owned `BifArguments` first (`src/runtime/interpreter.rs:826-827`).

The alias is not avoidable for a body that both reads an argument and changes the process. Safe Rust cannot hold `&Term` from a `&ProcessState` and `&mut ProcessState` of the same process. Thus the copy must stay for that kind of body, and only the bodies that do not use the scheduler can read a borrow.

## The design: two argument lanes

Decision: split the `bif` instruction path into two lanes. A leaf lane reads the arguments as a fully initialized array of references and runs a body that receives no scheduler. An owned lane copies the arguments and runs every other body, exactly as today. The two lanes share the operand checks, the match-context check, and the whole result handling.

The two lanes keep one interface rule: **a body that the `bifs!` table writes with the context pattern `_` receives no context binding, so it cannot reach the scheduler.** The compiler enforces that rule. The leaf lane holds the measured leaf set of the plan, and every other body stays on the owned lane.

### The borrowed argument view

`BifArguments` becomes a borrowed view. The current struct holds `terms: [Term; MAX_BORROWED_ARGUMENTS]` (`src/runtime/interpreter/bif.rs:314-320`). The new struct holds the same count of references:

```rust
/// The placeholder of a slot that no argument filled. It stays outside the
/// slice of [`BifArguments::with`].
static UNUSED_ARGUMENT: Term = Term::Nil;

pub(super) struct BifArguments<'a> {
    terms: [&'a Term; MAX_BORROWED_ARGUMENTS],
    len: usize,
}
```

The array is fully initialized, because `&Term` is `Copy` and `[&UNUSED_ARGUMENT; 3]` writes three 8-byte values. The array holds no `Term` value, so it needs no `Term::nil()` for a slot above the length and no `Arc` operation. `Term::Nil` is a unit variant, so the `static` is a constant expression, and `Term` is `Sync` (the compile-time check is at `src/term.rs:94-102`). The design uses no `unsafe`, no `MaybeUninit`, no `assume_init`, no `transmute`, and no partially initialized array.

Changes:

- `BifArguments` (`src/runtime/interpreter/bif.rs:314-320`): `terms: [Term; 3]` becomes `terms: [&'a Term; 3]`, and the struct takes the lifetime `'a`.
- `BifArguments::new` (`src/runtime/interpreter/bif.rs:330-335`): `std::array::from_fn(|_| Term::nil())` becomes `[&UNUSED_ARGUMENT; MAX_BORROWED_ARGUMENTS]`.
- `BifArguments::push` (`src/runtime/interpreter/bif.rs:351-354`): the parameter becomes `term: &'a Term`.
- `BifArguments::with` (`src/runtime/interpreter/bif.rs:367-369`): the body becomes `FnOnce(&[&'a Term]) -> R`.
- `read_bif_arguments` (`src/runtime/interpreter.rs:3156-3170`): the return type becomes `BifArguments<'a>`, the process and the operands share the lifetime `'a`, and `read_any` becomes `read_any_ref` (`src/runtime/interpreter.rs:2924-2929`). The count check against `MAX_BORROWED_ARGUMENTS` and the error text stay.

The new `read_bif_arguments` copies no term, and it writes no term. Its references point to the literal of the instruction or to the register of the process.

### The leaf lane

`table.rs` gets two items for the measured leaf set of the plan (`docs/interpreter-dispatch-plan.md:133,136`): `erlang:'+'/2`, `erlang:'-'/2`, `erlang:tuple_size/1`, `erlang:element/2`, `erlang:hd/1`, and `erlang:map_get/2`.

```rust
/// True for the argument identities that the leaf lane reads as a borrow.
pub(super) fn is_leaf(id: BifId) -> bool {
    matches!(
        id,
        BifId::ErlangPlus2
            | BifId::ErlangMinus2
            | BifId::ErlangTupleSize1
            | BifId::ErlangElement2
            | BifId::ErlangHd1
            | BifId::ErlangMapGet2
    )
}

/// Run a leaf BIF with borrowed arguments. The body of an entry of this
/// function reads no scheduler.
pub(super) fn dispatch_leaf(
    id: BifId,
    arguments: &[&Term],
) -> Result<BifResult, BeamError> {
    match (id, arguments) {
        (BifId::ErlangPlus2, &[left, right]) => {
            erlang::arith("+", left, right, i64::checked_add)
        }
        (BifId::ErlangMinus2, &[left, right]) => {
            erlang::arith("-", left, right, i64::checked_sub)
        }
        (BifId::ErlangTupleSize1, &[term]) => Ok(match term {
            Term::Tuple(elements) => BifResult::Value(Term::Integer(elements.len() as i64)),
            _ => BifResult::badarg(),
        }),
        // erlang:element/2, erlang:hd/1, and erlang:map_get/2 follow.
        (id, _) => {
            let (module, name, _) = mfa(id);
            Err(unresolved_mfa(module, name, arguments.len()))
        }
    }
}
```

The arm body keeps the table syntax. The slice pattern `[left, right]` against `&[&Term]` binds `left` and `right` as `&Term`, which is the same type that the owned `dispatch` binds (`src/runtime/interpreter/bif.rs:536-543`). Thus an arm moves from `table.rs` to `dispatch_leaf` with no change of its body except the context pattern, which becomes `_`.

A reference pattern in front of the slice pattern, `&[left, right]`, dereferences the `&[&Term]` and leaves a place of type `[&Term]`. The bindings then have the by-value mode, so `left: &Term`. A plain slice pattern against `&[&Term]` would instead bind `&&Term` under the match ergonomics. The scratch check below confirms the difference.

The leaf list is the single source of truth for the lane. `is_leaf` and `dispatch_leaf` hold the same identities. A unit test of `tests/runtime/interpreter/bif.rs` asserts that every leaf identity resolves from its MFA to the same `BifId` (`resolve`, `src/runtime/interpreter/bif.rs:488-495`) and that `dispatch_leaf` and `dispatch` give the same result for a representative argument list of each leaf. The test also asserts the fallback arm, so a missing leaf identity cannot pass.

### The call site

The `Instruction::Bif` arm (`src/runtime/interpreter.rs:800-866`) selects the lane from the resolved identity. The read and the body call become:

```rust
let result = if let Some(state) = process.length_trap.take() {
    self.bif_trap(scheduler, "erlang", "length", state)?
} else if is_leaf(*bif) {
    cursor.commit(process);
    let values = read_bif_arguments(process, arguments)?;
    let size = values.with(|terms| match_context_byte_size(*bif, arguments, terms))?;
    match size {
        Some(size) => BifResult::Value(size),
        None => values.with(|terms| table::dispatch_leaf(*bif, terms))?,
    }
} else {
    let values = read_owned_bif_arguments(process, arguments)?;
    cursor.commit(process);
    let size = values.with(|terms| match_context_byte_size(*bif, arguments, terms))?;
    match size {
        Some(size) => BifResult::Value(size),
        None => values.with(|terms| self.bif_slice(scheduler, *bif, terms))?,
    }
};
```

Four details follow from this code:

- `cursor.commit(process)` moves before the read on both lanes. The commit writes the already advanced `cursor.position` into the active frame, so the value is the same. The read of the current code happens before the commit, and the two errors that the read can report (an unset operand and a match context) leave the run, so the order of the commit changes no observable result. The design states the order explicitly because the borrow forces it.
- `read_owned_bif_arguments` is the current `read_bif_arguments` body with `read_any`, and it keeps the owned `BifArguments`. The struct of the owned lane keeps the old shape and the old cost, so an effectful `bif` instruction pays nothing for the new lane. The two structs need two names: `BifArguments<'a>` for the borrow and `OwnedBifArguments` for the copy.
- `match_context_byte_size` (`src/runtime/interpreter.rs:2999-3016`) becomes generic over the element type with the `std::borrow::Borrow<Term>` bound, so one function serves `&[&Term]` and `&[Term]`. The body changes `matches!(value, Term::MatchContext(_))` to `matches!(value.borrow(), Term::MatchContext(_))`.
- The leaf lane returns `BifResult` and not `Option<BifResult>`. The fallback arm of `dispatch_leaf` reports the unresolved MFA, exactly as the fallback arm of `dispatch` does (`src/runtime/interpreter/bif.rs:542-545`), and it is reachable in a unit test, so it keeps 100 percent line coverage. `is_leaf` keeps the non-leaf identities out of that arm in a run. A leaf body may return every variant of `BifResult`, including `Trap` and `TrapBif`, because a leaf body can build an owned argument vector itself.

The functions of the other paths do not change: `call_arguments` (`src/runtime/interpreter.rs:3182-3192`), `call_bif` (`src/runtime/interpreter/bif.rs:635-645`), `named_bif` (`src/runtime/interpreter/bif.rs:663-674`), `bif_slice` (`src/runtime/interpreter/bif.rs:695-703`), `bif_trap` (`src/runtime/interpreter/bif.rs:776-800`), `finish_bif` (`src/runtime/interpreter.rs:2199-2280`), `Scheduler::current_process_mut` (`src/runtime/scheduler.rs:995-1001`), and `ProcessState::registers` (`src/runtime/process.rs:217`).

### Why safe Rust accepts it

The leaf lane holds a shared borrow of the scheduler, through the process, and it calls a function that takes no scheduler. Rust accepts the shared borrow because no second borrow of the same value exists during the call. The order of the borrows is:

1. `cursor.commit(process)` takes `&mut ProcessState`, writes the frame position, and returns.
2. `read_bif_arguments(process, arguments)` takes `&ProcessState` and `&[Operand]` for the lifetime `'a` and returns `BifArguments<'a>`. The references point into the operands of the instruction and into the register file of the process.
3. `dispatch_leaf(*bif, terms)` takes `&[&Term]` and no scheduler, so the shared borrow of step 2 stays valid and no mutable borrow exists.
4. The owned lane is not reachable while `BifArguments<'a>` lives, because the two lanes are exclusive branches of one `if`.

The owned lane takes `&mut Scheduler` only after its own read returns an owned `OwnedBifArguments`. An owned value holds no borrow, so the mutable borrow of the scheduler is free.

The borrow checker proves the remaining facts:

- The `'a` of `BifArguments<'a>` is the shorter of the process borrow and the operand borrow, so no reference outlives the storage that holds the term.
- The leaf body cannot store a reference, because its return type `BifResult` holds owned values and no lifetime.
- The leaf body cannot reach the process, the scheduler, or any mutable runtime state, because the macro-generated call passes no context binding.

The scratch check of this section compiled the structure against the real types. The check changed `BifArguments` to the borrowed form, changed `read_bif_arguments` to `read_any_ref`, added a leaf arm for `erlang:'-'/2`, and built the workspace at `9adc310` in a scratch worktree with `mise exec -- cargo check --locked --lib`. The check passed with no `unsafe` and no error.

## Behavior of each path

Every path keeps its exact behavior. The table gives the pinned ERTS source that fixes each path, and the PBeam function that keeps it.

### The owned path

The owned path is an external call, an apply, or a `bif` instruction whose identity is not a leaf. ERTS passes the arguments of a BIF as a pointer into the X register file: `BIF_ALIST` is `Process* A__p, Eterm* BIF__ARGS, ErtsCodePtr A__I` (`erts/emulator/beam/bif.h:37`), `BIF_ARG_1` is `BIF__ARGS[0]` (`:46`), and the emulator calls the body as `result = (*bf)(c_p, reg, I)` (`erts/emulator/beam/emu/bif_instrs.tab:257`). PBeam copies the arguments into an owned vector, because several bodies store the arguments for a later turn. The owned path therefore does not match the ERTS argument form, and this design keeps it, because a body that changes the process cannot hold a borrow of it. `call_arguments` (`src/runtime/interpreter.rs:3182-3192`) keeps its `Vec<Term>` and its per-register error text. `call_bif` (`src/runtime/interpreter/bif.rs:635-645`) keeps the `&[Term]` parameter and the `Some(id)` to `bif_slice`, `None` to `named_bif` split.

### The trap path

A trap returns `BifResult::Trap` or `BifResult::TrapBif` with an owned `Vec<Term>`. ERTS keeps the trap state in the process and continues through the export: `$SAVE_CONTINUATION_POINTER($NEXT_INSTRUCTION); SET_I(c_p->i); $DISPATCH();` at `erts/emulator/beam/emu/bif_instrs.tab:296-306`. PBeam stores the same state in `PendingBif { module, function, arguments: Vec<Term>, tail }` (`src/runtime/process.rs:354-364`), and the next turn takes it and calls `bif_trap` (`src/runtime/interpreter.rs:578-587`, `src/runtime/interpreter/bif.rs:776-800`). The arguments of a trap are owned on both sides, so a trap is lane-independent: a leaf body that traps builds its own `Vec<Term>` exactly as the owned dispatch does today, and `finish_bif` handles the result (`src/runtime/interpreter.rs:2249-2263`). The design adds no trap, removes no trap, and changes no trap argument.

### The `pbeam_cap` path

`pbeam_cap` is a PBeam module and not an ERTS `bif.tab` entry, so no ERTS source fixes it. The path is the `resolved == None` arm of `call_bif` (`src/runtime/interpreter/bif.rs:643-645`), which reaches `named_bif` (`src/runtime/interpreter/bif.rs:663-674`) and then one body of `src/runtime/interpreter/bif/pbeam_cap.rs`. The bodies take `&mut Scheduler` and borrowed `&Term` arguments, for example `pbeam_cap::request(scheduler, kind, request)` (`src/runtime/interpreter/bif.rs:672-674`). The caller holds the arguments in an owned `Vec<Term>` from `call_arguments` or from `Program::bif` (`src/runtime/interpreter/bif.rs:606-618`), so the borrow of the arguments and the mutable borrow of the scheduler do not alias. The design does not touch this path. The capability behavior and its records are in [the effect inventory](effect-inventory.md) and [the capability security design](capability-security-design.md).

### The `length` trap

`length/1` is the only guard BIF with a trap (`docs/interpreter-dispatch-plan.md:30`). ERTS runs it as the `i_length` instruction, which enters `i_length_start__` and calls `erts_trapping_length_1` (`erts/emulator/beam/emu/bif_instrs.tab:147-150`, `:163-195`), and the trap export is `erlang:length/3` (`erts/emulator/beam/erl_bif_guard.c:47-55`, `:226-247`). PBeam keeps the trap state in `ProcessState::length_trap: Option<Vec<Term>>` (`src/runtime/process.rs:311`) and re-enters the body on the next turn through `self.bif_trap(scheduler, "erlang", "length", state)` (`src/runtime/interpreter.rs:823-824`). The `BifResult::TrapBif` arm of the `Bif` instruction stores the state, decrements the cursor, commits, and yields (`src/runtime/interpreter.rs:838-847`). The design keeps `length/1` on the owned lane in this task: `length` is not in the measured leaf set, so the trap path is byte-for-byte the current code. A later task that measures `length/1` adds it to the leaf set, and the trap handling stays outside the leaf lane either way, because it takes ownership of the arguments.

### The result handling

The result handling is identical on both lanes, because it runs after `values` has left the scope, on `scheduler.current_process_mut()` (`src/runtime/interpreter.rs:836-866`). ERTS fixes each branch:

- A value goes to `x(0)`: `if (ERTS_LIKELY(is_value(result))) { x(0) = result; CHECK_TERM(x(0)); $NEXT0(); }` (`erts/emulator/beam/emu/bif_instrs.tab:292-295`).
- A guard failure with a non-zero fail label jumps to the fail label and raises nothing: `$FAIL($Fail)` of `i_bif.call` (`erts/emulator/beam/emu/bif_instrs.tab:88-93`).
- A raise with fail label 0 enters the error handler: `I = handle_error(c_p, I, reg, &export->info.mfa)` (`erts/emulator/beam/emu/bif_instrs.tab:308-313`), which the `BifResult::Raise` arm of PBeam mirrors (`src/runtime/interpreter.rs:848-859`).
- The whole epilogue of a BIF or NIF is the same shape: `nif_bif.epilogue()` writes `x(0)`, does the return, and handles the trap and the error (`erts/emulator/beam/emu/bif_instrs.tab:604-636`).

The design changes no result variant, no fail-label jump, no exception class, no stack trace, and no reduction charge. The light-BIF reduction and the switch-out before the body stay on the external call path (`erts/emulator/beam/emu/bif_instrs.tab:227-235`), and the `bif` instruction charges no reduction on either lane, as today.

## The predicted gain and the confirming measurement

### Predicted gain

The design removes the buffer initialization and the argument copy of the leaf lane. The operand reads, the match-context check, the dispatch match, and the result handling stay. The prediction against the measured parts:

| Part | Measured base at `4261205` | Predicted after the leaf lane | Basis |
| --- | --- | --- | --- |
| `bif_arguments_build` | 24.883 ns | about 15 ns to 16 ns | the 6.811 ns of initialization leaves; the operand reads and the reference stores stay |
| `bif_slice_minus` | 5.900 ns | 5.900 ns | the identity dispatch and the subtraction do not change |
| `bif_result_value_handling` | 7.379 ns | 7.379 ns | the `BifResult` match and the `x` write do not change |
| one local `call_only` floor-loop iteration | 148.699 ns to 153.672 ns | about 138 ns to 145 ns | estimated: the floor loop holds the same `gc_bif '-'`, so the build part is on its critical path |
| the added `gc_bif '+'` on the same loop | 70.828 ns to 71.525 ns | about 60 ns to 63 ns | estimated from the same build part |

The scratch check measured the build part in a session of its own, with the borrowed `read_bif_arguments` and the owned one interleaved in one session at load 25.82, each number the minimum of nine timed runs of 100000 operations:

| Revision | `read_bif_arguments`, ns for one operation |
| --- | --- |
| base `9adc310` | 20.252, 22.483, 20.252 |
| borrowed `BifArguments<'a>` prototype | 11.062, 11.058, 11.060 |

The paired fall is 9.190 ns to 11.425 ns. The scratch numbers are a direction check and not a landed result: the session load is high, and the harness of the closed task #549 named the part `bif_arguments_build` with a different harness. A separate task measures the landed change in a quiet window.

The row `map_get/2` on a 1000-key map has a larger expected fall than the `+` row, because the copy of a compound argument is an `Arc` clone of about 10.3 ns for the clone and the drop (`docs/interpreter-dispatch-measurements.md:229`), against about 1 ns for the reference store. The design states that as an estimate, not as a measurement.

Two limits weaken the prediction. First, the whole BIF call path is 38.162 ns of one floor-loop iteration of about 150 ns, and the removable part is the build alone, so the floor-loop gain is a small number with the run-to-run noise of the benchmark beside it. Second, the estimate for the `+` row and the floor loop is a forecast from the measured parts, and the plan marks every forecast as an estimate.

The task gate of #549 is not part of this design. That gate asks for a fall of at least 16.963 ns, which is half of the measured 33.925 ns share. The removable part of the build is about 9 ns to 11 ns, because the operand reads stay and the dispatch and the result handling stay. Thus the borrowed interface alone does not meet that gate. If the owner keeps the gate, the parts that the leaf lane does not remove need their own task: the identity dispatch of `bif_slice_minus`, the arithmetic or term operation of the body itself, and the result handling of `bif_result_value_handling`.

### The measured target and the gate

The measured target is the borrowed path of the floor loop in `mise run bench-dispatch`, at 200000 iterations, in the local `call_only` shape and in the position shape.

- The target: the local `call_only` floor loop falls below the base group of the same session, and the fall is at least the fall of the build part that the micro-measurement reports. The plan gate of the floor loop is at most 150 ns for each iteration (`docs/interpreter-dispatch-plan.md:179`, `:188`).
- The second gate: the added `gc_bif '+'` of the same loop stays below 80 ns, which is the gate of step 5 (`docs/interpreter-dispatch-plan.md:175`).
- The rejection rule: the floor loop does not fall below the base group of the same session, or the micro-measurement shows no fall of the build part. Then the borrow does not remove the measured work, and the design stops.

### The measurement

Two commands confirm or reject the prediction.

1. The micro-measurement of the parts: `cargo test --locked --release --lib -- dispatch_attribution_costs -- --nocapture --test-threads=1` at `tests/runtime/interpreter/dispatch_attribution_costs.rs:284`. The test gains a `bif_arguments_build` row that measures the whole read of a `bif` instruction with two arguments, as [PR #561](https://github.com/fahchen/pbeam/pull/561) did, and it keeps the `bif_slice_minus` row (`:298`).
2. The floor loop and the added rows: `mise run bench-dispatch`, at 200000 and 20000 iterations, for the local `call_only` shape, the position shape, and the added rows of the measured set.

Both commands run in one quiet session, in the off-peak window that the record of #546 describes (`docs/interpreter-dispatch-measurements.md:264`), with three interleaved runs of the base revision and three of the change.

### The session control

The measurement must carry the control of its own session:

- The base group of the same session, interleaved with the change, at the same revision of the runtime code, is the control. The record names the revision of each group.
- The record gives the minimum of the nine sub-runs of each probe, and the spread of the three runs of each group.
- The record gives the one-minute load of every run and the host and toolchain of the session.
- A fixed historical floor-loop number is not a control. The base floor loop moved outside the BIF path: it was 161.754 ns with a range of about 161.5 ns to 162.2 ns at `c2f0341` (`docs/interpreter-dispatch-measurements.md:279`), and the `Term::eq` change of [#548](https://github.com/fahchen/pbeam/pull/548) made it 147.5 ns to 153.7 ns at `4261205` ([#549 comment](https://github.com/fahchen/pbeam/issues/549)). A record that compares its after number with a number of another revision cannot separate the change from the revision and the load.
- The record gives the paired difference of each round, as the `V5b` measurement of the attribution does (`docs/interpreter-dispatch-measurements.md:310`).

## Rejected alternatives

- **The `MaybeUninit` slot form.** It measured a floor-loop fall of 6.057 ns to 13.184 ns, and it was the only form of the current interface that removed the initialization. It needs `unsafe` in the array construction, the slice construction, and the release, and `AGENTS.md` forbids new `unsafe`. Rejected. [PR #561](https://github.com/fahchen/pbeam/pull/561) records the measurement and the closure.
- **`[const { Term::Nil }; 3]`, `std::array::from_fn`, `[Option<Term>; 3]`, and a `Vec` with capacity three.** Task #549 measured every one of them: neutral, slower, or slower. Rejected.
- **A split of the process state from the scheduler state.** Move the running process out of `Scheduler` (`src/runtime/scheduler.rs:153-238`) into a field of its own, so that a borrowed process and a mutable scheduler are disjoint. This does not remove the alias for the bodies that matter: a body that reads an argument and changes the process still needs `&Term` and `&mut ProcessState` of the same process at the same time, which safe Rust rejects. The change is also large, because `current_process_mut` (`src/runtime/scheduler.rs:995-1001`) has many callers. The leaf lane needs no scheduler at all, so the split adds cost and risk with no measured gain.
- **A `BifContext` that excludes the current process.** 89 bodies read or change the process through `cx.scheduler.current_process_mut()`, for example `binary::match_2` (`src/runtime/interpreter/bif/table.rs:41`). A context without the process changes every effectful body, and the leaf lane needs no context. Rejected.
- **One argument element type for both lanes (`&[&Term]` everywhere).** The owned lane would need a `Vec<&Term>`, which allocates on the `call_ext` path, and an effectful body still cannot hold the borrow while it changes the process. Rejected.
- **A `Vec<Term>` or a reusable owned vector as the leaf default.** It removes the allocation but keeps the copy of every argument, which is the measured cause. Rejected, as the plan records (`docs/interpreter-dispatch-plan.md:210`).
- **A register file of `Vec<Term>` with a `Term::EMPTY` sentinel.** The plan proposes the sentinel for step 8 (`docs/interpreter-dispatch-plan.md:101`), and step 8 is skipped (`docs/interpreter-dispatch-measurements.md:394`). The sentinel also does not give a `&[Term]` slice: the arguments of a `bif` instruction are separate operands, and an operand can be a literal of the instruction and not a register, so no contiguous slice of `Term` holds them without a copy.
- **A generated leaf lane from one `bifs!` entry marker.** A `@leaf` marker in the `bifs!` entry, with the macro that generates both dispatches, would keep one table. `macro_rules!` cannot combine the marker and the closure pattern of one entry in one repetition: the two optional groups have different lengths, and the build reports `meta-variable ... repeats 0 times, but ... repeats 1 time` (checked with the pinned rustc 1.99.0). The workarounds are a recursive macro that collects the entries, or a change of the table syntax of 553 entries. The first consumer needs six leaf arms, and a later task that extends the leaf set can revisit the generation with its own measurement.
- **A run-time classification table of leaf identities.** The plan's step 5 decided that the borrowed path follows the shape of the call and not a classification of BIFs (`docs/interpreter-dispatch-measurements.md:372`). A scan of a list for each call also adds work to the hot path. The leaf predicate is a compile-time `matches!` over six variants, and it is the only change to that decision that the measurement requires.
- **Every body on the leaf lane.** A body that changes the process cannot read a borrow of it, and a body that names the context binding must keep the owned path. Rejected.

## Limits and known gaps

- The measured leaf set is six BIFs. Every other pure BIF stays on the owned lane, so the design does not yet remove the copy of a `tuple_size/1` call through `call_ext` or of any body that the first task does not measure. The plan's step 5 leaves the `call_ext` of a pure BIF on the owned path as a known gap (`docs/interpreter-dispatch-measurements.md:380`).
- The design keeps the two lane lists in two places: the `bifs!` entry of `table.rs` and the leaf arm of `dispatch_leaf`. A unit test holds them equal, and a later task can fold them when the leaf set grows.
- The design does not measure the landed change. The scratch check of the borrow structure and the scratch micro-measurement are not a run of the design, and the confirming measurement needs a quiet session.
- The prediction for the floor loop and the `+` row is an estimate. The run-to-run noise of the benchmark is about 10 percent at 200000 iterations (`docs/interpreter-dispatch-measurements.md:385`), so a small fall needs the paired difference of the interleaved runs to be visible.
- The owned lane still copies each argument of an effectful `bif` instruction. This is inherent to safe Rust, and the design states it as a limit and not as a defect.
- The design does not change the `Arc` ownership model, the reductions, the traps, or the result handling. It adds no path that ERTS does not have: ERTS reads the arguments from the X registers and passes a pointer to the body (`erts/emulator/beam/bif.h:37,46`).
