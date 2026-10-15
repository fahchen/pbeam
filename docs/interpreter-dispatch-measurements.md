# Interpreter dispatch measurements

This document is the measurement record of the interpreter dispatch work. It is not a task tracker. GitHub Issues are the source of task status, and epic [#506](https://github.com/fahchen/pbeam/issues/506) holds the steps, their links, and their state. The plan is `docs/interpreter-dispatch-plan.md`.

The document collects the numbers that the merged pull requests of steps 0 to 6 report. The text names the revision and the source of each number. Every number is either measured or estimated, and the text says which one.

Baseline revision: `078c119` (2026-10-12). Current revision: `a0104b4`, the head of `origin/main` at the time of the cursor measurement of task [#550](https://github.com/fahchen/pbeam/issues/550). The runtime code of `a0104b4` equals the code of `c2f0341`, the revision of the attribution below, except for `src/term.rs` of [#548](https://github.com/fahchen/pbeam/pull/548), which removed the pair-stack allocation of `Term::eq`. Thus the floor-loop numbers of the earlier sections belong to their own revisions, and the section on the cursor re-measures the floor loop at `a0104b4`.

## The environment of the numbers

| Item | Value |
| --- | --- |
| Host | Darwin 27.0.0 arm64, Apple M1 Max, 10 CPUs |
| Rust | `rustc 1.99.0 (b940084d7 2026-09-28)`, `cargo 1.99.0 (5f94df478 2026-08-27)` |
| ERTS | OTP 29, ERTS 17.1, `jit`, `schedulers 10`, `logical processors 10` |
| Benchmark | `mise run bench-dispatch` at the release profile, `tests/integration/dispatch_baseline.rs` |
| Probe result check | Each probe runs on PBeam and on the pinned ERTS, and the target asserts that the two results are equal |
| Terra harness | `bash scripts/terra_latency.sh` on the example `terra_counter`, through a pseudo-terminal |

One number in a benchmark table is the minimum of nine runs of one probe inside one run of the benchmark. A column of a summary table aggregates the runs of the step that owns it, and each table states the aggregation rule and the revision of each column. The load of a run changes the number, so the text records the load of each group that a table uses.

## How to read the tables

- The floor loop holds `test`, `gc_bif '-'`, and one call. The benchmark measures three shapes: `call_ext_only` in `dispatch_probe`, `call_only` in `dispatch_probe`, and `call_only` in `dispatch_position_probe` after 20 guest functions.
- The added rows hold the floor loop of `dispatch_probe` plus one instruction: `gc_bif '+'`, the `bif` of `tuple_size/1`, the `bif` of `erlang:element/2`, the `bif` of `erlang:hd/1`, and the `bif` of `map_get/2` on a 1000-key map.
- A ratio is the PBeam number divided by the pinned ERTS number of the same run and the same row. A ratio above one means that PBeam is slower.
- A claim that a shape costs at most a fixed number of nanoseconds must name its session, because two sessions of one revision differ by more than the margin of the 120 ns gate. A claim that a change moved a shape needs one session that interleaves the two revisions.
- Each step column comes from the runs of the step that owns the column, with the aggregation rule that the table states. Two neighbouring columns therefore come from different runs or different groups of runs, and their difference holds both the change of the code and the run-to-run noise of about 10 percent at 200000 iterations.

## The interleaved check of the 120 ns gate of the term plan

Row 9 of the term performance plan, `docs/term-performance-design-codex-v2.md`, asks that the floor loop of the local shape costs at most 120 ns for each iteration. This is not the gate of step 9 of the dispatch plan, which asks for at most 150 ns. One measurement of one revision alone cannot check that gate, because the run-to-run noise at 200000 iterations is larger than the margin: a single session of the current `main` measured 122.0 and 122.3 ns for the two floor shapes, above the gate, and an interleaved session of the same revision measured 117.8 and 117.9 ns, below it.

The decisive session interleaves the after-revision of #585 (`d54e6a4`) with the current `main` (`ddb32f6e`, which holds the host-term boundary of #582, the live-byte harness of #589, the pseudo-terminal fix of #590, and the same-register map update of #597). Each round runs `mise run bench-dispatch` in one detached worktree for each revision, and a comparison uses only the two revisions of one round, so the load of the host reaches both. Each number is the minimum of nine runs inside one invocation.

| Round | `d54e6a4` `call_only`/position, 200000 | `ddb32f6e` `call_only`/position, 200000 | One-minute load, `d54e6a4`/`ddb32f6e` |
| --- | --- | --- | --- |
| 1 | 124.9 / 124.1 | 119.1 / 119.1 | 14.88 / 26.15 |
| 2 | 118.4 / 118.4 | 117.9 / 117.8 | 32.26 / 29.92 |
| 3 | 118.5 / 118.4 | 119.8 / 118.2 | 27.68 / 25.70 |
| Minimum | 118.4 / 118.4 | 117.9 / 117.8 | |

The session gives two results. The floor loop of the current `main` costs 117.9 ns and 117.8 ns as the minimum of the three rounds, and the after-revision of #585 costs 118.4 ns for both shapes, so both revisions pass the gate at their minimum across the rounds. Both revisions pass the gate in round 2 and in round 3, where every value is below 120 ns. Round 1 is the round that does not pass for the after-revision of #585, which measures 124.9 ns and 124.1 ns there. The three rounds do not establish the size of any difference between the two revisions, and the gate claim rests on the minimum across the rounds. The control of the same session, the `call_ext_only` floor loop, measures 285.7 ns for `d54e6a4` and 286.4 ns for `ddb32f6e` as the minimum, which is a difference of 0.7 ns. This session does not measure the earlier baseline, so it does not confirm or deny the layout effect of +15.6 ns that the step 9 session reported for this control.

## Step 0: the baseline

Pull request [#520](https://github.com/fahchen/pbeam/pull/520) (task [#508](https://github.com/fahchen/pbeam/issues/508)), merged as `211a645`. The measured revision is `078c119`, the head of that branch.

The step adds the benchmark target, the two probe fixtures, the runner `scripts/dispatch_baseline.sh`, and the mise task. It changes no runtime code.

Measured at 200000 iterations, in nanosecond for each iteration: the position shape is 506.8, the local `call_only` shape is 337.5, and the module-qualified `call_ext_only` shape is 564.0. The ERTS number of the same run is 2.52, 2.52, and 2.50.

Measured added cost of one instruction on the local `call_only` loop at 200000 iterations: `gc_bif '+'` 261.9 ns, the `bif` of `tuple_size/1` 250.1 ns, the `bif` of `element/2` 250.4 ns, the `bif` of `hd/1` 265.7 ns, and the `bif` of `map_get/2` on a 1000-key map 399.8 ns. The added numbers move by about 10 percent between runs of the same build.

Measured at 20000 iterations: the position shape is 504.0, the local shape is 339.0, and the module-qualified shape is 567.6.

The step also records the end-to-end key latency at `078c119`: 0.087 s to 0.105 s for the four keys on the release build, against 0.003 s to 0.008 s from the same harness run on the pinned ERTS. The plan quotes 3 ms for the ERTS key latency (`docs/interpreter-dispatch-plan.md:23`). The first frame is 1.36 s on the release build and 0.19 s on ERTS.

## Step 1: the loader side tables

Pull request [#521](https://github.com/fahchen/pbeam/pull/521) (task [#509](https://github.com/fahchen/pbeam/issues/509)), merged as `5bbb261`. The measured revision is `c06c513`, the head of that branch.

The loader builds `jump_targets` and `function_starts` once for each module, and the interpreter reads them. A jump, a taken fail label, a `catch` label, and each frame creation stop scanning the instruction stream. `max_x` does not land in this step, because no consumer exists before the register array of step 8.

Measured: the position shape falls from 498.9 ns to 327.8 ns at 20000 iterations, and from 505.6 ns to 326.2 ns at 200000 iterations. This is a fall of 34.3 percent and 35.5 percent. The position penalty of the earlier scan is gone: the position shape and the local shape then cost the same number, 326.2 against 326.7 at 200000 iterations.

Measured: the local `call_only` shape falls 2.2 percent, from 334.2 ns to 326.7 ns at 200000 iterations.

Measured: the module-qualified `call_ext_only` shape rises 2.6 percent, from 562.6 ns to 577.6 ns at 200000 iterations. Its function label is the first label of the module, where the removed scan read two instructions only, and one hash read costs more than those two instructions.

Measured: the first form of the two tables used the default `SipHash` of `HashMap`. That form measured 351.6 ns to 354.3 ns for each iteration of the local loop, above the 335.0 ns to 340.1 ns of the scan that it replaced. The loader owns the key set, so the tables do not need a hash that resists a chosen key. The FNV-1a mixer replaced `SipHash`, and the same measurement then gave 326.7 ns to 332.8 ns.

## Step 2: the execution cursor

Pull request [#523](https://github.com/fahchen/pbeam/pull/523) (task [#510](https://github.com/fahchen/pbeam/issues/510)), merged as `c284fa4`. The measured revision is `32930f0`, the head of that branch.

The run loop holds the active module and the next instruction index in a `Cursor`. A straight-line instruction advances the cursor. A call that enters a frame, a return, an exception that truncates the stack, a yield, and a caught error refresh the cursor. The loop commits the cursor to `Frame::next_instruction` before a body call and before it leaves the loop. The loop compares the integer `ModuleId` of the frame with the `ModuleId` of the cursor instead of comparing a module name `String` on each iteration.

Measured: about −14 percent on the two `call_only` shapes. The local shape falls from 331.3 ns to 284.5 ns, and the position shape falls from 329.6 ns to 287.2 ns, at 200000 iterations. The position shape falls 12.9 percent and the local shape falls 14.1 percent.

Measured: −3.1 percent on the module-qualified `call_ext_only` shape, from 576.9 ns to 559.2 ns at 200000 iterations.

Measured: the added rows fall −15.6 percent to −24.8 percent. `gc_bif '+'` falls from 226.5 ns to 176.9 ns, `tuple_size/1` from 209.0 ns to 157.1 ns, `element/2` from 204.4 ns to 172.6 ns, `hd/1` from 200.6 ns to 163.7 ns, and `map_get/2` from 288.9 ns to 240.2 ns.

The load of the measurement session was 13 to 23. The runs are interleaved in pairs, and one further interleaved sequence at load 34 to 55 agrees on the direction and the size of the gain.

## Step 3: the frame and export name IDs

Pull request [#524](https://github.com/fahchen/pbeam/pull/524) (task [#511](https://github.com/fahchen/pbeam/issues/511)), merged as `516ceb1`. The measured revision is `8cfab38`, the head of that branch.

A frame holds the `ModuleId` of its module and a `NameId` in the place of two `String` values, and an export key is `(NameId, u32)` in the place of `(String, u32)`. The name table of the module holds each function name one time, and the code table gives the name back when a diagnostic or a stack trace needs it.

Measured: −12.0 percent on the local `call_only` shape, from 283.3 ns to 249.2 ns at 200000 iterations. Measured: −12.8 percent on the position shape, from 286.1 ns to 249.5 ns. Measured: −10.2 percent on the module-qualified shape, from 556.7 ns to 500.0 ns.

Measured: the call path allocates no `String`. The count of allocations for each iteration of the local shape falls from 4 to 2, the position shape from 4 to 2, and the module-qualified shape from 5 to 2. The `String` part falls from 2, 2, and 3 to 0 in each shape. The two allocations that stay are the pair stack of `Term::eq` (`src/term.rs:318`) and the argument `Vec<Term>` of the `gc_bif` path.

Measured: `Frame` changes from 88 bytes to 40 bytes.

The two remaining allocations are not `String` values, and this step leaves both open. The argument `Vec<Term>` is the subject of step 5. The pair stack of `Term::eq` belongs to no step of the plan.

## Step 4: the BIF identity at load time

Pull request [#525](https://github.com/fahchen/pbeam/pull/525) (task [#512](https://github.com/fahchen/pbeam/issues/512)), merged as `1e86242`. The measured revision of the column is `d244d8f`, the head of that branch, which holds the runtime code of the fixed head `aa4c0e7`.

The loader resolves the MFA of a `bif` import and of a `call_ext` import to a `BifId` one time. A `bif` instruction holds no module name and no function name, and the run of that instruction reads no table. A `call_ext` of a BIF reaches the body with no binary search of the 553-entry `BIFS` table.

The review rejected the first head `486c0a5`, because a `call_ext` of module code still fell through to `call_mfa`, which resolved the MFA again. The fixed head `aa4c0e7` sends that call through `call_export`, which searches no `BIFS` entry.

Measured on the fixed head `aa4c0e7`: the position shape is 176.9 ns and the module-qualified shape is 303.5 ns for each iteration at 200000 iterations, against 249.8 ns and 499.9 ns at `516ceb1`. The local `call_only` shape is 177.9 ns against 249.6 ns.

Measured attribution of the 196.4 ns that the module-qualified external call loses: about 72 ns come from the `bif` identity of the same loop, which holds the identity in the place of the name, and about 124 ns come from two failed 553-entry binary searches of `resolve` that the external call path held at `516ceb1`, one in `call_external` and one in `call_mfa`. The first head `486c0a5` measured 358.3 ns for the same shape, so the fall-through of the `call_mfa` path cost 54.8 ns of that number.

Measured added cost of one instruction on the fixed head: `gc_bif '+'` 105.4 ns, `tuple_size/1` 85.5 ns, `element/2` 100.5 ns, `hd/1` 87.0 ns, and `map_get/2` 177.2 ns. The gate of the step is a BIF delta below 200 ns, and each row is below it.

Measured run-time table work of one `call_ext` of module code: two failed binary searches of `BIFS` before, and one scan of the 5-entry `PBEAM_BIFS` after. The scan of the `call_ext_only` shape measures 1.24 ns. The largest single scan of any call path measures 3.22 ns, which is the `HEAVY_BIFS` scan of 7 entries for the reduction class. A `call_ext` of a PBeam BIF runs both scans, and a scratch run measured 5.41 ns for that path. The step records that path as a known gap.

## Step 5: the borrowed fixed-arity BIF arguments

Pull request [#526](https://github.com/fahchen/pbeam/pull/526) (task [#513](https://github.com/fahchen/pbeam/issues/513)), merged as `ea4dd13`. The measured revision is `8385d12` for the library and `9ef5c97` for the allocation test.

A `bif` instruction no longer builds a `Vec<Term>` for its arguments. It reads the argument operands into `BifArguments`, which holds at most the three terms of a `gc_bif3` instruction on the stack, and it passes the borrowed slice to the BIF body. The owned path stays for every call that is not a `bif` instruction.

Measured: the pure leaf path allocates no `Vec`. The added allocation count of `gc_bif '+'`, of the `bif` of `tuple_size/1`, of `element/2`, and of `hd/1` falls from 1 to 0 for each iteration. The count of the `call_only` floor loop itself falls from 2 to 1, and this step does not own that row.

Measured: the `+` delta falls from 107.2 ns to 77.7 ns for each iteration. The gate of the step is a `+` delta below 80 ns, and the three after runs give 77.7 ns, 78.8 ns, and 78.2 ns.

Measured: the floor shapes move 3.5 ns to 11.1 ns for each iteration. That range is inside the run-to-run noise of this benchmark, and the step claims no reduction for the floor shapes. The after runs had a load of 23 to 27 against 1.96 to 2.68 for the before runs, so the after floor numbers are a floor and not a best case.

Measured: the `tuple_size/1` added row moves by 0.9 ns, and the change is inside the noise.

## Step 6: the decoded instruction form

Pull request [#527](https://github.com/fahchen/pbeam/pull/527) (task [#514](https://github.com/fahchen/pbeam/issues/514)), merged as `d36f686`. It changes no runtime behavior, so it adds no column to the tables below.

Measured with the pinned rustc 1.99.0: `Instruction` is 136 bytes, `Operand` is 40 bytes, `Term` is 40 bytes, and `BifId` is 4 bytes. Each arm of `Instruction` has the 136-byte stride of the largest arm, `BinaryGetBinary`, whose fields take 132 bytes. One iteration of a floor loop retires 4 instructions and reads 544 bytes of slots inside an 816-byte range. One iteration of an added-instruction loop retires 5 instructions and reads 680 bytes inside a 1088-byte range. The loader writes `TestYield` at the function entry and the loop returns to that entry, so the four are the `TestYield`, the clause test, the subtraction, and the `call_only`; `records_the_decoded_loop_body` (`tests/runtime/interpreter.rs`) asserts those counts, and `FLOOR_LOOP_RECORDS` (`tests/integration/dispatch_baseline.rs`) is four.

Measured: the run loop performs no byte-code decode on a visit, because the loader decodes the whole `Code` chunk one time. The only encoding-dependent work of a visit is the bounds check of the instruction index.

Measured estimate of the memory-system difference of the two record strides, as a proxy: the proxy runs as its own step of the benchmark script, without the dispatch benchmark beside it, and each number is the minimum of five runs. At 300000 records of 136 bytes, which is 40800000 bytes and the estimated size of one Terra redraw instruction stream, the difference is 2.09 ns for each record, or 8.38 ns for each floor-loop iteration with 4 retired instructions. That number is 5.0 percent of the measured 167.4 ns of one `call_only` floor-loop iteration.

The decision of the step follows the step 8 gate of the plan: a compact form must give at least 10 percent on the floor loop and must measure at most 32 bytes for each instruction. The 5.0 percent estimate is below the gate, and the 816-byte range of the floor loop is below 1.3 percent of the smaller L1 data cache of the host, so the loop is taken to be cache-resident. The step therefore keeps the decoded form and skips step 8. The residency is an assumption, and the host has no hardware counter to confirm it.

## Step 10: the reference-count share

Task [#544](https://github.com/fahchen/pbeam/issues/544) measures the count of the reference-count changes on the hot path, and the share of the time that they take. Step 10 ([#518](https://github.com/fahchen/pbeam/issues/518)) has a 20 percent gate in the plan (`docs/interpreter-dispatch-plan.md:152`): "If counts remain below the gate, keep `Arc` and report the remaining dispatch cost." Task [#544](https://github.com/fahchen/pbeam/issues/544) states the same falsifier: if the counts stay below 20 percent of loop time, stop and report the ceiling. Step 9 closed without an implementation, so the baseline is the current `main`.

The measured revision is `a4072f1`, the head of `origin/main` at the time of the runs. All numbers of this section come from one session on 2026-10-15 from 08:29 to 08:33 JST, on the host of the environment table, at a one-minute load of 1.46 to 4.21. The floor-shape counts are the exception: a rerun at 08:48 JST recorded them with a revision and load header (`target/arc-probe-floor-rerun.txt`), and its rows are the same as the rows of the first run (`target/arc-probe-floor.txt`, which has no header). No other `cargo`, `rustc`, or `nextest` process ran at the start of each run.

### The method

A temporary counting patch adds four process-wide counters: the `Arc` clones and the `Arc` drops of a `Term` handle, and the `Rc` clones and the `Rc` drops of the module handle of the run-loop cursor. The patch is not committed, and the pull request of the task holds it. It adds no `unsafe`. It counts these sites:

- `Clone for Term` and `Drop for Term` (`src/term.rs:572`, `src/term.rs:517`), when the term holds an `Arc`: a binary, a match context, and a tuple, a list cell, a fun, or a map whose `Arc` is still present.
- `Term::take_nested` and `Term::take_group` (`src/term.rs:443`, `src/term.rs:476`), which release the `Arc` of a tuple, a list cell, a fun, or a map and leave no `Arc` for the later drop. Thus the drop of one handle counts one time.
- The `Rc` clone of `Cursor::new` (`src/runtime/interpreter.rs:2775`), once for each entry of the run loop, and the `Rc` clone and drop of `Cursor::refresh` (`src/runtime/interpreter.rs:2798`) when the active module changes. The drop of the cursor counts one `Rc` drop.

The run loop gives each change to the instruction kind that ran when the change happened. A change outside the run loop goes to the row `<outside the run loop>`. Two consumers read the counters:

- A temporary test in `tests/integration/dispatch_baseline.rs` runs each floor shape at 20000 and 200000 iterations on the release build and prints the rows. The count for each iteration is the count at 200000 less the count at 20000, divided by 180000.
- The `read_nif` of `prim_tty` writes the rows to the file of `PBEAM_ARC_PROBE` at each non-empty read of the terminal. Thus a block of the file holds the work from the read of one key to the read of the next key. The block holds the work of the key and of its idle time of 2 s (`KEY_GAP`, `scripts/terra_latency.py:33`), so each block is an upper bound for its key. The last key, `q`, has no block, because no read follows it.

The time and the cost numbers come from the clean tree of `a4072f1` in the same session, before the patch. The patched build is slower, so this section uses no latency of the patched build.

### The count for each instruction of the floor shapes

The rows are the release build of `a4072f1` with the counting patch, at 200000 iterations. The record is `target/arc-probe-floor-rerun.txt`, from 2026-10-15 08:48:52 JST, at a load of `{ 4.61 7.20 7.23 }` before and after the run. Each shape retires the same 4 instructions for each iteration: `TestYield`, `TestExactEqualOperands`, `Bif` (`gc_bif '-'`), and the call.

| Shape | Instruction kind | Runs at 200000 | `Arc` clones | `Arc` drops | `Rc` clones | `Rc` drops |
| --- | --- | --- | --- | --- | --- | --- |
| `call_ext_only`, `dispatch_probe` | `TestYield` | 200001 | 0 | 0 | 0 | 0 |
| | `TestExactEqualOperands` | 200001 | 0 | 0 | 0 | 0 |
| | `Bif` | 200000 | 0 | 0 | 0 | 0 |
| | `CallExternal` | 200000 | 0 | 0 | 0 | 0 |
| | `<outside the run loop>` | — | 0 | 0 | 51 | 51 |
| `call_only`, `dispatch_probe` | `TestYield` | 200001 | 0 | 0 | 0 | 0 |
| | `TestExactEqualOperands` | 200001 | 0 | 0 | 0 | 0 |
| | `Bif` | 200000 | 0 | 0 | 0 | 0 |
| | `CallOnly` | 200000 | 0 | 0 | 0 | 0 |
| | `<outside the run loop>` | — | 0 | 0 | 51 | 51 |
| `call_only`, position probe | `TestYield` | 200001 | 0 | 0 | 0 | 0 |
| | `TestExactEqualOperands` | 200001 | 0 | 0 | 0 | 0 |
| | `Bif` | 200000 | 0 | 0 | 0 | 0 |
| | `CallOnly` | 200000 | 0 | 0 | 0 | 0 |
| | `<outside the run loop>` | — | 0 | 0 | 51 | 51 |

At 20000 iterations each shape gives the same rows with 0 `Arc` changes and 6 `Rc` pairs outside the run loop. Thus each floor shape makes 0 `Arc` clones and 0 `Arc` drops for each iteration, and (51 − 6) / 180000 = 0.00025 `Rc` pairs for each iteration. The `Rc` pairs are the `Cursor::new` of each entry of the run loop after a yield, and not work of an iteration. Every value of the loop is an immediate integer, and an atom holds only its index since [#619](https://github.com/fahchen/pbeam/pull/619), so no term of the loop holds a reference count. The result agrees with the static reading of the four handlers and with the attribution above, which says that the floor loop performs no `Arc` clone.

### The count for each Terra key press

The release and the debug builds of the patch give the same rows in every block. The raw files are `target/arc-probe-terra.txt` (release, load `{ 2.10 2.72 4.33 }`) and `target/arc-probe-terra-debug.txt` (debug, load `{ 1.93 2.67 4.29 }`).

| Block | Instructions | `Arc` clones | `Arc` drops | `Rc` clones | `Rc` drops |
| --- | --- | --- | --- | --- | --- |
| Start and first frame, to the read of the first key | 1718917 | 483480 | 678235 | 37563 | 37562 |
| Key `j` | 259688 | 76371 | 84892 | 23807 | 23807 |
| Key `j` | 253558 | 74381 | 81986 | 23734 | 23734 |
| Key `k` | 253558 | 74381 | 81986 | 23734 | 23734 |

One key press makes about 0.33 `Arc` drops for each instruction. The instruction kinds with the most changes in the first key block:

| Instruction kind | Runs | `Arc` clones | `Arc` drops | `Rc` clones | `Rc` drops |
| --- | --- | --- | --- | --- | --- |
| `Move` | 87727 | 51761 | 39035 | 0 | 0 |
| `CallExternal` | 11560 | 6592 | 7910 | 10612 | 10612 |
| `CallFun` | 3556 | 4716 | 6103 | 3544 | 3544 |
| `Deallocate` | 4214 | 0 | 10629 | 0 | 0 |
| `GetMapElements` | 10381 | 4556 | 2927 | 0 | 0 |
| `PutTuple` | 4507 | 659 | 3945 | 0 | 0 |
| `GetList` | 2347 | 1939 | 3327 | 0 | 0 |
| `CallLast` | 2249 | 0 | 3174 | 0 | 0 |
| `Return` | 14739 | 0 | 0 | 9592 | 9592 |

`Move` makes 68 percent of the `Arc` clones of the key. The `Rc` pairs come from `Cursor::refresh` at a call into another module and at a return.

### The cost of one change and the time of the loop and of a key

Measured on the clean tree of `a4072f1` in the same session. The release costs are `mise run bench-term`, the record is `target/term-costs.txt`, and the load is `{ 2.44 2.73 4.57 }`. The debug costs are the same test at opt-level 0, which is the profile of the debug `pbeam` binary, and the record is `target/term-costs-debug.txt` at a load of `{ 2.33 2.70 4.55 }`. Each cost is the minimum of 9 timed runs.

| Cost of one clone and drop | Release | Opt-level 0 |
| --- | --- | --- |
| `Arc` of one term, the largest of the six payload kinds of `prints_the_cost_of_a_term_operation` (a 1000-key map in each column) | 11.111 ns | 33.734 ns |
| `Arc` of a tuple | 9.600 ns | 25.129 ns |
| `Rc` of one term, the largest of the measured payload kinds | 3.962 ns, the fun payload of `prints_the_cost_of_a_fun_payload`, the largest of five kinds (tuple, binary, list cell, fun payload, map payload) | 14.269 ns, the tuple, the largest of three kinds (tuple, binary, list cell), because only `prints_the_cost_of_a_term_operation` ran at opt-level 0 |
| Bare `Arc` | 9.475 ns | 12.145 ns |
| Bare `Rc` | 3.821 ns | 13.016 ns |

The floor loop is `mise run bench-dispatch` at a load of `{ 1.46 2.56 4.61 }`, the record is `target/dispatch-baseline.txt`, and each number is the minimum of nine runs of one probe at 200000 iterations: `call_ext_only` 257.8 ns, local `call_only` 101.9 ns, and the position shape 102.2 ns.

The key latency is `bash scripts/terra_latency.sh`, three release runs at a load of `{ 4.21 3.16 4.67 }` to `{ 3.48 3.06 4.60 }` and three debug runs at a load of `{ 3.16 3.01 4.55 }` to `{ 2.39 2.85 4.46 }`. The record is `target/terra-latency-544.txt`. The release keys are 0.014 s to 0.039 s, and the release first frame is 0.34 s to 0.56 s. The debug keys are 0.130 s to 0.156 s, and the debug first frame is 1.13 s to 1.31 s.

### The share

The share is an estimate for the counted sites at cache-resident costs, and not an upper bound. It uses this form: (max(`Arc` clones, `Arc` drops) × the largest `Arc` cost + max(`Rc` clones, `Rc` drops) × the largest `Rc` cost) / time. The time of a key is the shortest key of all runs, and the count of a key is the largest key block. Each choice makes the share larger. The sites that the patch does not count (section 5) and a cold count line can make the real share larger.

| Item | Count | Time | Time of the changes | Share |
| --- | --- | --- | --- | --- |
| `call_ext_only` floor loop, one iteration | 0 `Arc`, 0.00025 `Rc` | 257.8 ns | 0.001 ns | 0.0004 percent |
| `call_only` floor loop, one iteration | 0 `Arc`, 0.00025 `Rc` | 101.9 ns | 0.001 ns | 0.001 percent |
| Position floor loop, one iteration | 0 `Arc`, 0.00025 `Rc` | 102.2 ns | 0.001 ns | 0.001 percent |
| Release key press | 84892 `Arc`, 23807 `Rc` | 14 ms | 1.038 ms | 7.4 percent |
| Debug key press | 84892 `Arc`, 23807 `Rc` | 130 ms | 3.203 ms | 2.5 percent |
| Release start and first frame | 678235 `Arc`, 37563 `Rc` | 0.34 s | 7.685 ms | 2.3 percent |
| Debug start and first frame | 678235 `Arc`, 37563 `Rc` | 1.13 s | 23.416 ms | 2.1 percent |

The atomic part of the release key press is the `Arc` count times the bare `Arc` cost less the bare `Rc` cost: 84892 × (9.475 − 3.821) ns = 0.480 ms, or an estimated 3.4 percent of the 14 ms key for the counted sites at cache-resident costs. A change to non-atomic counts can save about that part of the counted sites. At opt-level 0 the bare `Arc` pair measured below the bare `Rc` pair, so the debug build shows no atomic part.

The micro costs are uncontended and the count line is in the cache. A count line that is cold in the Terra redraw costs more, so the shares can be low by that cache effect. The largest costs, the largest counts, and the shortest key make the shares larger, which offsets a part of that effect.

### The decision

Every share is below the 20 percent gate, and every share is below half of the gate. The floor loop makes no `Arc` change, so step 10 has no floor-loop gain to take. For the counted sites at cache-resident costs, the release key press spends an estimated 7.4 percent in the counts, and an estimated 3.4 percent in the atomic part of the counts. The uncounted sites add about 1 to 2 percent more counts (section 5), so the estimate stays below half of the gate. Thus the task keeps `Arc` and proposes no ownership-path change. Step 10 ([#518](https://github.com/fahchen/pbeam/issues/518)) has no measured reason to start. The owner decides its state.

## 1. The cumulative effect on the floor shapes

The number in each cell is nanosecond for each iteration, and the ratio in each cell is that number divided by the pinned ERTS number that the step's own pull request pairs with it. The aggregation rule of each column, and the revision of its run:

- step 0 `078c119`, [#520](https://github.com/fahchen/pbeam/pull/520): one benchmark run; each number is the minimum of nine probe runs of one probe.
- step 1 `c06c513`, [#521](https://github.com/fahchen/pbeam/pull/521): the pull request records three interleaved benchmark runs of each revision, and each number is the minimum of nine probe runs of one probe.
- step 2 `32930f0`, [#523](https://github.com/fahchen/pbeam/pull/523): three interleaved benchmark runs of each revision; the column is the minimum of the three runs, and each run is the minimum of nine probe runs.
- step 3 `8cfab38`, [#524](https://github.com/fahchen/pbeam/pull/524): four interleaved benchmark runs of each revision; the column is the minimum of the four runs, and each run is the minimum of nine probe runs.
- step 4 `aa4c0e7`, [#525](https://github.com/fahchen/pbeam/pull/525): four interleaved benchmark runs of `d244d8f`, which holds the runtime code of `aa4c0e7`; the column is the minimum of the four runs, and each run is the minimum of nine probe runs. The before values of the same step are the minimum of thirteen interleaved runs at `516ceb1`.
- step 5 `8385d12`, [#526](https://github.com/fahchen/pbeam/pull/526): three interleaved benchmark runs of each revision; the column is the minimum of the three runs, and each run is the minimum of nine probe runs.

Each column comes from its own group of runs, and each group is a different session with a different host load, so a column boundary is a change of run group and of step together.

| Shape | iterations | step 0 `078c119` | step 1 `c06c513` | step 2 `32930f0` | step 3 `8cfab38` | step 4 `aa4c0e7` | step 5 `8385d12` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `call_ext_only`, `dispatch_probe` | 20000 | 567.6 ns, 272.9x | 577.3 ns, 283.0x | 553.6 ns, 264.9x | 507.6 ns, 244.0x | 304.1 ns, 146.2x | 296.4 ns, 142.5x |
| `call_only`, `dispatch_probe` | 20000 | 339.0 ns, 146.1x | 328.8 ns, 149.5x | 285.2 ns, 129.6x | 251.4 ns, 114.3x | 179.8 ns, 82.9x | 169.4 ns, 77.0x |
| `call_only`, position probe | 20000 | 504.0 ns, 209.1x | 327.8 ns, 145.7x | 290.4 ns, 126.8x | 251.6 ns, 109.9x | 179.7 ns, 84.4x | 168.7 ns, 74.6x |
| `call_ext_only`, `dispatch_probe` | 200000 | 564.0 ns, 225.6x | 577.6 ns, 236.7x | 559.2 ns, 225.5x | 500.0 ns, 205.8x | 303.5 ns, 125.9x | 292.0 ns, 118.2x |
| `call_only`, `dispatch_probe` | 200000 | 337.5 ns, 133.9x | 326.7 ns, 129.6x | 284.5 ns, 112.5x | 249.2 ns, 98.9x | 177.9 ns, 70.6x | 166.7 ns, 65.9x |
| `call_only`, position probe | 200000 | 506.8 ns, 201.1x | 326.2 ns, 128.9x | 287.2 ns, 110.9x | 249.5 ns, 97.8x | 176.9 ns, 69.9x | 167.0 ns, 65.2x |

The load of each run group: step 0 `{ 5.05 6.37 5.81 }`, step 1 `{ 3.39 5.09 5.73 }`, `{ 3.72 5.05 5.70 }`, and `{ 3.30 4.87 5.62 }` for the after runs, step 2 13 to 23, step 3 7.06 to 17.54 for the after runs, step 4 8.64 to 12.67 for the fixed head, and step 5 23.32 to 27.02 for the after runs. The step 5 after numbers are therefore a floor under a high load, and the step 4 numbers come from a quieter host. A reader must not treat a fall from the step 4 column to the step 5 column as a measured gain of step 5, because the step 5 run had the higher load.

The ratio of the position shape at 200000 iterations falls from 201.1 at step 0 to 65.2 at step 5. The ratio of the module-qualified shape falls from 225.6 to 118.2 over the same range.

## 2. The cumulative effect on the added instructions

The number in each cell is the added cost of one instruction for each iteration on the local `call_only` loop of `dispatch_probe` at 200000 iterations. Each column comes from the same runs as the column of the same step in section 1, with the same aggregation rule and the same revision. The ratio of a column uses the ERTS added number that the step's own pull request pairs with the added PBeam number. For step 3, [#524](https://github.com/fahchen/pbeam/pull/524) prints the ERTS number of the full added row and the ERTS number of the `call_only` floor loop of the same after runs, so the ratio uses their difference (3.17 ns less 2.52 ns gives 0.65 ns for `tuple_size/1`).

| Added instruction | step 0 `078c119` | step 1 `c06c513` | step 2 `32930f0` | step 3 `8cfab38` | step 4 `aa4c0e7` | step 5 `8385d12` |
| --- | --- | --- | --- | --- | --- | --- |
| `gc_bif '+'` | 261.9 ns | 229.2 ns | 176.9 ns | 178.0 ns | 105.4 ns | 77.7 ns |
| the `bif` of `tuple_size/1` | 250.1 ns, 384.8x | 208.6 ns, 372.5x | 157.1 ns, 241.7x | 177.1 ns, 272.5x | 85.5 ns, 129.5x | 81.8 ns, 141.0x |
| the `bif` of `element/2` | 250.4 ns | 199.7 ns | 172.6 ns | 181.4 ns | 100.5 ns | 83.0 ns |
| the `bif` of `hd/1` | 265.7 ns | 195.3 ns | 163.7 ns | 176.9 ns | 87.0 ns | 78.5 ns |
| the `bif` of `map_get/2`, 1000-key map | 399.8 ns, 88.5x | 295.9 ns, 65.5x | 240.2 ns, 53.3x | 249.9 ns, 55.4x | 177.2 ns, 39.3x | 155.5 ns, 35.0x |

Each ratio is the PBeam added number divided by the ERTS added number of the same run and the same row. The ERTS added number of `gc_bif '+'`, `element/2`, and `hd/1` measures −0.08 ns to 0.12 ns across the step pull requests, and its sign and size change between runs. The added cost of those instructions therefore sits inside the run-to-run noise of the benchmark, and the subtraction of two noisy numbers, the added row less the floor loop, is unstable. Those rows have no ratio, and the rows whose ERTS added number is larger than that spread carry one. The ERTS added number is 0.56 ns to 0.66 ns for `tuple_size/1` and 4.44 ns to 4.52 ns for `map_get/2`; the 0.56 ns value comes from step 1, where the ratio of 372.5 divides 208.6 ns by 0.56 ns. The `tuple_size/1` ratio rises from 129.5 to 141.0 between step 4 and step 5, because the PBeam number falls and the ERTS number falls more. The `map_get/2` ratio falls from 39.3 to 35.0.

Step 3 measured its added rows in the same process as the full shape tables, and the `Frame` size change moved the layout of the code. Its added rows are therefore not a clean measure of one added instruction, and the step says so. The `map_get/2` row of step 3 moved by +13 ns in one run and by −3 ns in another run.

The HAMT walk of `map_get/2` keeps its measured cost of about 34 ns from the baseline, and no step changes it.

## 3. The end-to-end measurement at the current revision

Command: `bash scripts/terra_latency.sh release target/terra-latency-progress.txt`. The first measured revision is `ea4dd13`, and the second is `ea3e28a`; each was the head of `origin/main` at the time of its run. The raw output is `target/terra-latency-progress.txt`. The harness appends each block with its label, the revision, the time, and the host load.

The harness runs each mode one time. The numbers below are one run of each mode, and not the minimum of several runs, so they carry the run-to-run variance that a single run has.

| Mode | Revision | Load | First frame | Key `j` | Key `j` | Key `k` | Key `q` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| PBeam release | `ea4dd13` | `{ 4.14 4.57 6.08 }` | 0.57 s | 0.018 s | 0.038 s | 0.023 s | 0.039 s |
| PBeam release | `ea3e28a` | `{ 6.31 7.12 10.28 }` | 0.55 s | 0.035 s | 0.015 s | 0.021 s | 0.015 s |
| PBeam release | `e74a1af9` | `{ 12.19 25.63 26.34 }` | 0.52 s | 0.033 s | 0.014 s | 0.014 s | 0.015 s |
| Pinned ERTS, `erl +S 1` | `ea4dd13` | `{ 3.67 4.45 6.00 }` | 0.18 s | 0.007 s | 0.003 s | 0.007 s | 0.004 s |

The four key latencies of the release build are 0.018 s to 0.039 s at `ea4dd13`, 0.015 s to 0.035 s at `ea3e28a`, and 0.014 s to 0.033 s at `e74a1af9`. The three runs overlap, and each is one run of each mode, so they do not establish a run-to-run variance and they do not isolate the dispatch work. The run at `e74a1af9` also ran at a one-minute load of 12.19, above the 12 that this document uses for a measurement, so it cannot support a controlled comparison with the other two runs. The dispatch changes of that interval, the term equality without the pair stack (#548), the frame module and the cursor (#569), the classified operand form (#560), and the resolved jump and frame targets (#585), carry their own before-and-after tables in their pull requests; this document holds the floor-loop tables of the plan steps, the cost of one operation by kind, and the attribution of one instruction, and the attribution section is the stronger evidence for what one iteration spends. The pinned ERTS numbers of the same harness run are 0.003 s to 0.007 s. The first frame is 0.57 s on the release build and 0.18 s on ERTS.

This is the only end-to-end effect in this document that a reader can compare with other work. The harness sends the four keys two seconds apart (`KEY_GAP = 2.0`, `scripts/terra_latency.py:33`), which is the spacing of the design run below. The design record of the copy-on-write epic, `docs/copy-on-write-terms.md:292`, reports 0.13 s to 0.21 s for the same four keys at a revision that it does not name. The baseline record of step 0 reports 0.087 s to 0.105 s at `078c119`. The current measurement is below both ranges.

The measurement covers three revisions and one host. It does not isolate the dispatch work from the term work, the input handling, or the render, and the plan states that the redraw is 99.96 percent of one key press (`docs/interpreter-dispatch-plan.md:23`). It is one run of each mode, so it is weaker evidence than the minimum-of-nine floor-loop numbers above, and three single runs cannot separate the work of a revision from the variance of the host. The PBeam runs took a load of `{ 4.14 4.57 6.08 }`, `{ 6.31 7.12 10.28 }`, and `{ 12.19 25.63 26.34 }`, and the pinned ERTS run took `{ 3.67 4.45 6.00 }`; none is an idle host, and the third is above the bound that this document uses.

## The cost of one operation, by kind

This section answers one question: what does one operation cost, by kind? The four tables below hold the measured cost of one interpreter instruction, of one part of the call path, of one term operation, and of one end-to-end run. Each table names its revision and its source. Each number is measured, or the table marks it as computed.

### The interpreter instructions

The local `call_only` floor loop at 200000 iterations is the reference. The step 0 column is the before run of [#521](https://github.com/fahchen/pbeam/pull/521) at `211a645`, the merge of [#520](https://github.com/fahchen/pbeam/pull/520). Each number is the minimum of the nine sub-runs of a probe over three interleaved benchmark runs, and the load of the three runs is `{ 2.41 4.95 5.69 }`, `{ 3.57 5.07 5.71 }`, and `{ 3.53 4.96 5.66 }`. The current column is the run of [#527](https://github.com/fahchen/pbeam/pull/527) at `ea4dd13` at load `{ 3.67 4.38 5.93 }`, and each number is the minimum of the nine sub-runs of a probe.

| Item | Step 0 `211a645` | Current `ea4dd13` |
| --- | --- | --- |
| The floor loop, for each iteration (the plan shape at the step 0 revision, and the local `call_only` shape at the current revision) | 505.6 ns | 167.4 ns |
| The average for each instruction of the loop (computed: the floor-loop number divided by the 4 instructions that one iteration retires) | about 126.4 ns | about 41.9 ns |
| The added delta of `gc_bif '+'` | 248.9 ns | 75.6 ns |
| The added delta of the `bif` of `tuple_size/1` | 238.1 ns | 80.0 ns |
| The added delta of the `bif` of `erlang:element/2` | 242.7 ns | 81.8 ns |
| The added delta of the `bif` of `erlang:hd/1` | 253.3 ns | 77.3 ns |
| The added delta of the `bif` of `map_get/2` on a 1000-key map | 358.9 ns | 159.2 ns |
| The whole module-qualified `floor_ext_loop` (`call_ext_only`), for each iteration | 562.6 ns | 289.3 ns |

The step 0 floor-loop number is the plan shape, the `call_only` floor loop of `dispatch_position_probe` after 20 guest functions. The current floor-loop number is the local `call_only` shape of `dispatch_probe`. The two shapes cost the same number after step 1, and [#527](https://github.com/fahchen/pbeam/pull/527) records the plan shape at 167.7 ns at `ea4dd13`.

### The call-path parts

Each row is one part of one call or of one instruction. The "before" column gives the measured cost at the revision that held the part, and the "now" column gives the state at the current revision.

| Part of the call path | Measured cost before | State now |
| --- | --- | --- |
| The two `String` allocations of one call ([#524](https://github.com/fahchen/pbeam/pull/524)) | 45.8 ns to 49.7 ns for one pair, over three runs, each the minimum of 9 runs of 2000000 pairs | zero on the call path |
| The BIF argument `Vec<Term>` ([#526](https://github.com/fahchen/pbeam/pull/526)) | 31.9 ns for each BIF (`docs/interpreter-dispatch-plan.md:124`) | zero on the pure leaf path; `Instruction::CallExternal` of a pure BIF keeps one `Vec<Term>` |
| The 553-entry `BIFS` binary search of one BIF call ([#525](https://github.com/fahchen/pbeam/pull/525)) | one search at run time; the whole `is_bif` lookup is 76.5 ns for each call (`docs/interpreter-dispatch-plan.md:123`) | zero at run time |
| The two failed 553-entry `BIFS` searches of the external call path ([#525](https://github.com/fahchen/pbeam/pull/525)) | about 124 ns together | zero |
| The `function_at_label` scan of one frame creation ([#521](https://github.com/fahchen/pbeam/pull/521)) | about 12 ns for each preceding guest function, computed from the dummy-function series of [#520](https://github.com/fahchen/pbeam/pull/520): 385 ns at 6 preceding functions, 486 ns to 501 ns at 14, 535 ns at 18, and 672 ns at 30 | zero |
| The module-name `String` comparison for each instruction ([#523](https://github.com/fahchen/pbeam/pull/523)) | one comparison for each instruction | zero; the loop compares the integer `ModuleId` |
| The two small classification scans that stay ([#525](https://github.com/fahchen/pbeam/pull/525)) | `PBEAM_BIFS` with 5 entries at 1.24 ns for each scan, and `HEAVY_BIFS` with 7 entries at 2.80 ns to 3.22 ns for each scan | a `call_ext` of module code pays the `PBEAM_BIFS` scan, a `call_ext` of a BIF pays the `HEAVY_BIFS` scan, and a PBeam BIF path pays both at about 5.4 ns |

### The term operations

The numbers come from the committed measurement of `mise run bench-term` at `ec935af`, the commit that moves the deep-copy container outside the timer, release build, on the host of the environment table, at load `{ 8.08 13.95 14.04 }`. Each measured number is the minimum of 9 timed runs of one operation, the record is `target/term-costs.txt`, and the test asserts no time. A deep-copy number holds the copy, the push of the copy into a container of the run, and the storage that the copy allocates itself; the container is allocated before the timer and cleared between runs, so no allocation of the container is in the timed region, and the release of the copies is outside it. The drop of an immediate is the measurement of `mise run bench-drop` at the same revision, at load `{ 10.56 14.36 14.19 }`, whose record is `target/drop-costs.txt`. Step 7 merged as `c6cfb3a` and changed the term drop path, so `src/term.rs` and `src/term/map.rs` differ from `a6b838c`; the table reports the cost at the merged revision. The measurement replaces the earlier scratch numbers of the design measurement at `a6b838c`: 11.2 ns, 4.03 ns, 9.46 ns, 32.3 ns, 5102 ns, 132197 ns, 1811 ns, 58.4 ns, 28.9 ns, and 139840 ns.

| Term operation | Cost for one operation | Kind |
| --- | --- | --- |
| `Arc` clone and drop of one term | 10.3 ns; the payload does not change it (a binary 10.1 ns, a fun with two free variables 10.5 ns, one cons cell 10.3 ns, and a tuple that holds a 1000-key map 10.3 ns) | measured |
| `Arc` clone and drop of a shared map | 12.3 ns; about 2.0 ns above the tuple, which is inside the 3 ns gate of step 7 | measured |
| The placeholder allocation of `Map::drain_into` (`src/term/map.rs:503`) in the drop of a shared map, which step 7 ([#535](https://github.com/fahchen/pbeam/pull/535)) removed at `c6cfb3a` | none at `ec935af`; about 21 ns at `6fa5194` | computed before the removal: 32.199 ns less 11.377 ns; removed |
| `Rc` clone and drop of the same plain payload | 3.90 ns to 4.06 ns (a tuple 3.897 ns, a binary 3.910 ns, one cons cell 3.926 ns, a fun with two free variables 4.058 ns, and a shared map 4.001 ns) | measured |
| Bare `Arc` clone and drop | 9.801 ns | measured |
| Bare `Rc` clone and drop | 3.979 ns | measured |
| The two atomic operations of the `Arc` pair | about 5.8 ns | computed: 9.801 ns less 3.979 ns |
| Deep copy of a 1000-element tuple | 2542 ns | measured |
| Deep copy of a 1000-key map, which walks every node of the trie | 18968 ns | measured |
| Deep copy of a 102400-byte binary | 1728 ns | measured |
| The copy rate of the 102400-byte binary | 59.3 GB/s | computed: 102400 bytes divided by 1728.250 ns |
| Deep copy of a fun with two free variables | 26.7 ns | measured |
| Deep copy of one cons cell | 11.8 ns | measured |
| Deep copy of a tuple that holds a 1000-key map | 20027 ns | measured |
| Drop of an immediate | 2.26 ns | measured by `mise run bench-drop` at `ec935af` |

### The end-to-end runs

The first two rows are the record of step 0 ([#520](https://github.com/fahchen/pbeam/pull/520)) at `078c119`. The last two rows are the record of section 3 of this document at `ea4dd13`, the head of `origin/main` at the time of that run, and they are historical: step 6 (`d36f686`) changes no runtime code, but step 7 ([#535](https://github.com/fahchen/pbeam/pull/535)) changes the term drop path, and the harness has not measured the end-to-end latency again after step 7. A later change re-measures them.

| Mode | Revision | First frame | Key latency (the four keys) |
| --- | --- | --- | --- |
| PBeam release | `078c119` | 1.36 s | 0.087 s to 0.105 s |
| Pinned ERTS, `erl +S 1` | `078c119` | 0.19 s | 0.003 s to 0.008 s |
| PBeam release | `ea4dd13` | 0.57 s | 0.018 s to 0.039 s |
| Pinned ERTS, `erl +S 1` | `ea4dd13` | 0.18 s | 0.003 s to 0.007 s |

The design measurement at `b52e297` splits one redraw frame: `render` 39.0 ms and `patch` 43.4 ms, against 0.25 ms for the patch walk on the pinned ERTS. The repository has not re-measured this split on PBeam, so the two numbers are the design record and not a run of the harness above.

The four tables above have three limits. First, an added delta is measured inside one run against the floor loop of the same run, so it holds that instruction plus a small loop difference. Second, a call-path part is a cost for each call or for each instruction, and it is not a share of an end-to-end run; the parts overlap, so the sum of a column is not the cost of one key press. Third, the term table comes from the measurement at `ec935af`, whose term code is the code of `c6cfb3a`, and the end-to-end table comes from `078c119` and `ea4dd13`, so the two tables are not from the same revision.

## The cost of one instruction, by part

This section answers the question of issue [#546](https://github.com/fahchen/pbeam/issues/546): for the floor loop of `fixtures/dispatch/dispatch_probe.erl`, and for one instruction of it, how much of the cost belongs to each part?

The measured runtime revision is `c2f0341`. The later commits change this document and the harness only, so the measured code is the code of `c2f0341`. The base runs are on the clean tree of that revision, and each variant run is on the same tree with the scratch edit of the variant, which the record of the run marks with the line `the working tree differs from HEAD`. The measurements ran in the quiet off-peak window of 2026-10-13 08:49 to 08:58 JST on the host of the environment table above.

The harness of that session recorded the load before the release build of each run, so the load of a run is the load at the start of the build and not the load of the timed section, which the build raised. The corrected harness in `scripts/dispatch_attribution.sh` builds the test binaries first, waits for the one-minute load to fall to 12 or below, and then records the clock, the JST time, and the load. The `V5b` measurement of a second session used the corrected harness.

### The reference points

The harness measures two numbers in every build:

- `L`, the local `call_only` floor loop of `dispatch_probe` at 200000 iterations, in nanosecond for each iteration. The number is the minimum of the nine sub-runs of the probe.
- `I`, the added cost of one `gc_bif '+'` on the same loop at 200000 iterations, in nanosecond for each iteration. The number is the `plus_loop/2` number of the same run less `L` of that run.

Measured at `c2f0341`, the three base runs and their minimum and spread:

| Number | Run 1 | Run 2 | Run 3 | Minimum | Spread |
| --- | --- | --- | --- | --- | --- |
| `L`, ns for each floor-loop iteration | 162.929 | 161.754 | 162.723 | 161.754 | 1.175 |
| `I`, ns for each added `gc_bif '+'` | 70.643 | 72.136 | 71.281 | 70.643 | 1.493 |

One iteration of the floor loop retires the 4 instructions `TestYield`, `test is_eq_exact`, `gc_bif '-'`, and `call_only`. The function enters at the entry label, the loader writes `TestYield` after it, and `label_index` gives the position after the fail label. Thus the table below gives the share of one iteration for each part, and the same number divided by those 4 instructions for one instruction of the loop.

An ablation of a part that both `floor_loop` and `plus_loop` hold is not a reliable attribution of `I`, because the two loops do not retire the same instructions: `plus_loop` retires five against the four of `floor_loop`, so `V2` removes a further comparison of the module and `V3` removes operand checks and a register resize from the added `+`. A change of either kind can therefore move `I`. The measured `I` of `V2` to `V6` is 67.774 ns to 72.373 ns against the base 70.643 ns, which is inside the 1.493 ns spread of the base `I` plus the noise of one run, so these runs do not resolve the contributions to `I`. The column "one instruction" is a computed division of the measured iteration number and not a second measurement.

### The method

The main method is an ablation. A scratch build of `src/runtime/interpreter.rs` or of `src/term.rs` neutralises one sub-step, and the harness measures `L` and `I` in that build. The share of the sub-step is the difference between the variant and the base build of the same session. The scratch variants are not committed, and this section names each one.

A micro-measurement covers a part that an ablation cannot separate. The micro-measurement is `prints_the_cost_of_every_instruction_part` in `tests/runtime/interpreter/dispatch_attribution_costs.rs`, which reads the private helpers beside the interpreter. Each micro number is the minimum of nine timed runs of 100000 operations, in nanosecond for one operation. The thirteen numbers below come from the three base runs; each number is the minimum of the three runs, and the spread of the three runs is at most 0.421 ns.

| Variant | Scratch patch | Scratch edit | Part |
| --- | --- | --- | --- |
| base | none | none | The reference points `L` and `I` |
| `V2` | `V2-cursor-compare.patch` | Remove the per-iteration compare `current_frame(process).module_id != cursor.module.id` (`src/runtime/interpreter.rs:603`), because the floor loop keeps one module. | The module of the active frame and the cursor |
| `V3` | `V3-operand-checks.patch` | Read the X slot with no bound check and no set check in `read_operand` (`src/runtime/interpreter.rs:3986-3989`), and write the X slot with no `resize` in `write_operand` (`src/runtime/interpreter.rs:4023-4027`). | The operand and register access |
| `V4` | `V4-label-index.patch` | Index the `function_starts` table by the label number with a `Vec` in `src/beam/module.rs`, so `function_at_label` (`src/runtime/interpreter.rs:3119`) reads a slot instead of hashing the label with FNV-1a. | The function lookup of a frame creation |
| `V5` | `V5-inline-bif.patch` | Run `erlang:'-'/2` on two immediate integers inline in the `Bif` arm (`src/runtime/interpreter.rs:800-866`), so the loop skips `read_bif_arguments`, `match_context_byte_size`, `bif_slice`, and the `BifResult` match. The edit also removes `cursor.commit(process)` and replaces the checked subtraction with a wrapping subtraction, so it is a combined measurement. | The `Bif` call path of the loop, which holds the call overhead, the cursor commit, and the arithmetic check |
| `V6` | `V6-term-eq-immediate.patch` | Return from `Term::eq` (`src/term.rs:353`) before the pair stack when the two terms are both integers, both `nil`, or both atoms, and keep the heap pair stack for a deeper term. | The comparison pair stack |
| `V5b` | `V5-inline-bif-isolated.patch` | The same inline path, with `cursor.commit(process)` and the checked subtraction kept, so the difference holds the call overhead only. | The BIF call overhead, isolated |

The dispatch and the branch have no variant. A build cannot remove the fetch of the record or the match of the opcode without a different loop, so the micro-measurement `dispatch_fetch_and_match` gives a lower bound. No variant removes the `label_index` lookup of the `test` fail label, so that lookup is present in the loop and is not separated by an ablation.

### The measurement

The record of one build is one run of the harness, and every run records the clock, the JST hour, and the load. The measurement is three interleaved runs of the base build and of each variant in one quiet session. The reported number of a cell is the difference of the minimum of the three variant runs and the minimum of the three base runs, and the spread is the largest minus the smallest per-round difference `base run less variant run`.

The recorded load of every run is at or below 11.69, and the value is the load before the release build of that run. The base runs recorded `{ 3.98 5.53 7.97 }`, `{ 10.81 7.88 8.63 }`, and `{ 9.59 9.29 9.10 }`. The variant runs recorded `{ 8.41 7.26 8.41 }` to `{ 11.69 9.73 9.36 }`. The load rose above 12 between runs, because the release build of each variant was the only heavy work on the host, so a later run of a build waited for the one-minute load to fall to 10 or below before it measured.

The `V5b` measurement is a second session at the same runtime revision, and it used the corrected harness, which records the load after the build. A session control belongs to the revision of that session. The control of this section is the base floor loop of `c2f0341`. The second session measured 161.465 ns, 162.190 ns, and 161.761 ns, a range of 161.5 ns to 162.2 ns, which is within 1 percent of the 161.754 ns of the first session, so the session is accepted. The loads of the second session, recorded after each build, are `{ 11.69 10.09 10.23 }`, `{ 9.96 9.81 10.12 }`, and `{ 11.70 10.22 10.26 }` for the base build, and `{ 10.97 9.99 10.19 }`, `{ 11.59 10.14 10.23 }`, and `{ 11.05 10.13 10.22 }` for the `V5b` build. Every one-minute load of that session is at or below 11.70.

Task [#549](https://github.com/fahchen/pbeam/issues/549) measured the base floor loop at `4261205` over five runs: 149.364 ns, 151.053 ns, 150.023 ns, 150.859 ns, and 153.976 ns. In the same order, the load before each run is `{ 5.74 10.77 12.62 }`, `{ 5.84 10.71 12.58 }`, `{ 5.61 10.58 12.53 }`, `{ 5.61 10.58 12.53 }`, and `{ 5.61 10.58 12.53 }`. Each run is below the control of `c2f0341`: the largest run, 153.976 ns, is 4.6 percent below the 161.465 ns of the second session, and the smallest, 149.364 ns, is 7.9 percent below the 162.190 ns of that session. The cause is the `Term::eq` change of [#548](https://github.com/fahchen/pbeam/pull/548), which lowered the floor loop outside the BIF call path. Thus a reader cannot use the range of `c2f0341` as the control of a later revision.

### The parts

The number in each cell is measured, or the cell reads `not measured`. A part with no ablation stays `not measured` for the iteration and for the instruction, and the micro-measurement column gives the cost of one operation of that part.

| Part | Sub-step | Method | ns for one floor-loop iteration | share of the iteration | ns for one instruction of the loop (computed: the iteration number divided by 4) | Run-to-run spread of the iteration number | Micro-measurement, ns for one operation |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1. The dispatch and the branch | The fetch of the record, the match of the opcode, and the branch | Micro-measurement; no ablation | not measured | not measured | not measured | not measured | `dispatch_fetch_and_match` 1.506 |
| 2. The operand and register access | The operand read, the X write, the bound check, and the set check | Ablation `V3` | -2.499 | -1.5 percent | -0.625 | 2.000 | `read_operand_x` 2.014, `read_operand_literal` 1.654, `write_operand_x` 5.750 |
| 3. The lookups | The module of the active frame and the cursor | Ablation `V2` | 9.182 | 5.7 percent | 2.296 | 4.474 | none |
| 3. The lookups | The function lookup of a frame creation | Ablation `V4` | 1.762 | 1.1 percent | 0.441 | 1.700 | `function_at_label` 2.678, `frame_creation` 6.340 |
| 3. The lookups | The label lookup of the `test` fail label | Present in the loop; no ablation removes it | not measured | not measured | not measured | not measured | `label_index` 2.418 |
| 3. The lookups | The export lookup and the BIF identity | Not present in the run loop; the loader resolves them one time | not measured | not measured | not measured | not measured | none; the lookup cost is not measured |
| 4. The state | The frame position | Inside `V2` and `V5`; not separated | not measured | not measured | not measured | not measured | none |
| 4. The state | The reduction accounting and the switch-out check | Not present in the floor loop | not measured | not measured | not measured | not measured | `light_bif_switch` 0.426 |
| 4. The state | The process state and `fcalls` | Present in the loop through the `TestYield` instruction; no ablation removes it | not measured | not measured | not measured | not measured | none |
| 5. The BIF call overhead | The `Bif` call path of the loop's `gc_bif '-'`: the borrowed argument path, the identity dispatch, and the result match, with the cursor commit and the checked subtraction kept | Ablation `V5b`, isolated | 33.925 | 21.0 percent | 8.481 | 0.784 | `read_bif_arguments` 20.255; `bif_slice_minus` 5.815, which is the dispatch of the resolved `BifId` plus the subtraction and not a lookup |
| 6. The term work | The comparison pair stack of `Term::eq` | Ablation `V6` | 13.257 | 8.2 percent | 3.314 | 6.893 | `term_eq_integer` 21.522 |
| 6. The term work | The `Arc` clone and drop | Not present in the floor loop | not measured | not measured | not measured | not measured | `term_clone_drop_shared_tuple` 9.610, `term_clone_drop_immediate` 3.472 |

The measured rows sum to 34.5 percent of one iteration: 21.0 percent for the `V5b` row, 8.2 percent for the comparison pair stack, 5.7 percent for the module of the active frame and the cursor, 1.1 percent for the function lookup, and -1.5 percent for the operand and register access. The combined variant `V5` gives 34.606 ns, or 21.4 percent, as an upper bound that also holds the cursor commit and the arithmetic check; the two edits change the number by 0.681 ns, which is inside the 0.784 ns spread of `V5b`. The rest of one iteration is the part with no ablation, the label lookup, the frame position, the `fcalls` decrement and the yield check of the `TestYield`, the body of the `test` and of the `call_only` after the ablations, and the arithmetic of the `gc_bif '-'`. Those parts are not measured, so the sum of the table is not the cost of one iteration.

What a neutralised sub-step may hide, for each row:

- Part 1: The micro-measurement walks the instruction records of a module that holds the floor-loop shape with a four-arm match, and it measures 1.506 ns for one operation. The run loop matches about 100 arms and holds the branch history of the loop, so the micro number is below the real dispatch cost. No ablation removes the fetch, the match, or the branch, so the part is not measured.
- Part 2: The `V3` ablation reads the X slot with no bound check and no set check and writes the X slot with no `resize`. It measured -2.499 ns for each iteration, and the three per-round differences are -3.530 ns, -2.219 ns, and -1.530 ns, so every run shows a slowdown of the loop, and the 2.000 ns spread is the range of those differences. That slowdown is not the cost of the two checks: the ablation also changes the code layout and the handling of the register file, so the check cost is not measured. The error path of the two checks never runs in the floor loop, so the ablation measures the check and the tag test and not the error text. The floor loop writes only x0 of a register file that holds x0 and one more slot, so the `resize` is already a no-op and the growth path of a new register is not measured.
- Part 3, the module and the cursor: `V2` removes the compare and the read of the active frame from the process together, so the 9.182 ns holds both and not the compare alone. The loop must keep the compare for a call that enters another module, because the refresh of the cursor reads the active frame. The micro-measurement does not separate one store.
- Part 3, the function lookup: `V4` replaces the FNV-1a `LabelMap` of `function_starts` with a `Vec` indexed by the label number, so the frame creation reads a slot instead of hashing the label. The `Vec` read and the read of the function vector stay, so the 1.762 ns is the hash and the probe and not the whole lookup. The micro-measurement confirms the size: `function_at_label` falls from 2.678 ns in the base build to 1.355 ns in the `V4` build, and `frame_creation` falls from 6.340 ns to 3.552 ns.
- Part 3, the label lookup: The `test is_eq_exact` fails for each nonzero counter and resolves fail label 6 with `label_index` (`src/runtime/interpreter.rs:1052`), which hashes the label in the `jump_targets` map. The lookup runs once for each iteration, and no variant removes it, so the part is present in the loop and not separated by an ablation. The micro-measurement gives 2.418 ns for one `label_index` call.
- Part 3, the export lookup and the BIF identity: The loader resolves the identity of the `bif` import one time, so the run of that instruction reads no table and the loop makes no export lookup and no BIF identity resolution. The cost of those two lookups is not measured. The micro-measurement `bif_slice_minus` gives 5.815 ns for one call of `bif_slice` with an already resolved `BifId`, so that number is the table dispatch of the identity plus the integer subtraction, and it is not a lookup.
- Part 4: The `Bif` arm and the `CallOnly` arm of the run loop do not call `light_bif_switch`; only the `call_ext` path and `send` do. Thus the floor loop has no reduction accounting and no switch-out check of a light BIF, and the `return` at the end of the loop charges one reduction for 200000 iterations. The fourth instruction of the iteration, `TestYield`, does touch the process state every iteration: it decrements `process.fcalls` and checks it against zero, and it yields when the count reaches zero (`src/runtime/interpreter.rs:1669`). The floor loop of the benchmark runs 200000 iterations with the yield budget of the run, so the check runs and the branch is not taken; no ablation removes it, and the part is present in the loop and not measured. Its cost is inside the unattributed remainder. The micro-measurement `light_bif_switch` gives 0.426 ns for one operation of the path that has it. `V2` removes a read of the active frame and a comparison of the module, and it does not remove a store of the frame position. The `cursor.commit` of `V5` is the edit that removes a write of the frame position, and no micro-measurement separates that write.
- Part 5: `V5b` inlines `erlang:'-'/2` and keeps `cursor.commit(process)` and the checked subtraction, so its 33.925 ns holds the call overhead only. It removes the `BifArguments` build, which the micro-measurement gives as 20.255 ns for one call of `read_bif_arguments`; the `match_context_byte_size` check; the `bif_slice` dispatch to the table entry, which the micro-measurement gives as 5.815 ns for one call; and the `BifResult` match. The operand read and the register write stay. The three per-round differences are 33.925 ns, 33.171 ns, and 33.141 ns. The first variant `V5` is a combined measurement that also removes the cursor commit and the checked subtraction, so its 34.606 ns is an upper bound; the two extra edits change the number by 0.681 ns, inside the spread.
- Part 6, the pair stack: `V6` returns from `Term::eq` before the pair stack when the two terms are both integers, both `nil`, or both atoms. It removes the allocation for the `test` of the floor loop and for each other immediate comparison, and it keeps the heap pair stack for a deeper term. The micro-measurement confirms it: `term_eq_integer` falls from 21.522 ns in the base build to 2.026 ns in the `V6` build. The loop difference is 13.257 ns and is below the 21.522 ns of one `Term::eq`, so part of the allocation cost is hidden by the pipeline of the loop. The spread of 6.893 ns is the largest of the table, and the per-round difference is 6.558 ns to 13.451 ns.
- Part 6, the `Arc`: The floor loop holds immediate terms, so it performs no `Arc` clone. The micro-measurements give 9.610 ns for the clone and drop of a shared tuple and 3.472 ns for the clone and drop of an immediate.

The issue says that a part with a large share becomes its own task. The measured shares above 5 percent are the BIF call overhead at 21.0 percent, the comparison pair stack at 8.2 percent, and the module of the active frame and the cursor at 5.7 percent. Task [#541](https://github.com/fahchen/pbeam/issues/541) owns the pair stack. The module of the active frame and the cursor is the residue of step 2. This section records the numbers; the owner decides which of the two unowned shares becomes a task.

## The active module and the execution cursor

This section answers the question of issue [#550](https://github.com/fahchen/pbeam/issues/550): what does the active module and the execution cursor add to one floor-loop iteration, and what part of that does the change of the task remove?

The runtime revision of the measurement is `a0104b4`, the head of `origin/main` at the time of the runs. Its runtime code equals the code of `c2f0341`, the revision of the attribution above, except for `src/term.rs` of [#548](https://github.com/fahchen/pbeam/pull/548), which removed the pair-stack allocation of `Term::eq`. Task [#549](https://github.com/fahchen/pbeam/issues/549) records that change as the cause of the lower base floor loop. The branch of the task holds the change of task #550 in its working tree, and `scripts/dispatch_attribution.sh` marks each run of that branch with the line `the working tree differs from HEAD`.

### The three parts

The cursor of step 2 holds three parts that touch the active module and the position. The micro-measurement of `tests/runtime/interpreter/dispatch_attribution_costs.rs` measures each part one operation at a time. Each number is the minimum of nine timed runs of 100000 operations, and the minimum of nine runs of the measurement, at a load of `{ 4.13 ... }` to `{ 10.31 ... }`. The build is the release build of the task branch. The `cursor_module_compare` row measures the per-iteration comparison that the change removes, so that row is a part of the base loop and not of the changed loop.

| Part | Operation | ns for one operation |
| --- | --- | --- |
| The module identity comparison | The read of the active frame and the compare of its `ModuleId` with the `ModuleId` of the cursor (`cursor_module_compare`) | 0.495 |
| The refresh on a module change | `Cursor::refresh` when the active module is the module of the cursor (`cursor_refresh_same_module`) | 11.818 |
| The refresh on a module change | `Cursor::refresh` when the active module is another module (`cursor_refresh_changed_module`), less the reset of the cursor module (`cursor_module_reset`) | 41.053 less 4.110 |
| The commit at each exit | `Cursor::commit` (`cursor_commit`) | 0.507 |

The refresh is the largest of the three parts in isolation, and the floor loop does not run it: the local `call_only` loop keeps one module, and only a call into another module, a return, and an exception that truncates the stack can change the active module. The comparison and the commit are each about 0.5 ns in isolation. The floor-loop result below shows which part the loop pays.

### The floor loop at `a0104b4`

The number is `L` of the attribution above: the local `call_only` floor loop of `dispatch_probe` at 200000 iterations, the minimum of the nine sub-runs of one run. The method is the ablation of the attribution. Three builds run interleaved in one session at a load of `{ 4.13 9.40 11.99 }` to `{ 4.42 9.90 12.24 }`: the base `a0104b4`, the `V2` ablation (the per-iteration comparison and the frame read removed), and the task change (the same removal, and each exit that can change the active module refreshes the cursor). Six runs of each build:

| Build | The six `L` values, ns | Minimum, ns | Fall from the base, ns |
| --- | --- | --- | --- |
| base `a0104b4` | 148.586, 149.597, 149.649, 148.948, 150.066, 150.213 | 148.586 | — |
| `V2` ablation | 144.596, 144.802, 145.496, 145.250, 145.757, 145.767 | 144.596 | 3.990 |
| task change | 154.478, 145.197, 145.254, 146.178, 146.154, 145.952 | 145.197 | 3.389 |

The first run of the task change is an outlier: its load is the same as the other runs of its round, and the other five runs of the change are at or below 146.178 ns. The minimum of the six runs does not use it. The per-round difference of the change less the base is −4.400 ns to −2.770 ns without that run, and the per-round difference of `V2` less the base is −4.795 ns to −3.698 ns.

A fourth build removes only the pre-body `cursor.commit(process)` of the `Instruction::Bif` arm, so that the arm reads no frame and writes no position. Three runs of the commit-removed build and three runs of the base of the same session, at a load of `{ 6.59 12.25 13.24 }` to `{ 11.02 13.60 13.78 }`:

| Build | The three `L` values, ns | Minimum, ns |
| --- | --- | --- |
| base | 149.746, 149.141, 151.479 | 149.141 |
| commit removed | 150.100, 155.865, 149.711 | 149.711 |

The commit-removed build is 0.570 ns above its base. The pre-body commit has no measured share of the floor loop, because the loop reads the committed position nowhere: the `call_only` arm replaces the whole frame at the end of each iteration, and the trap and raise paths of the `Bif` arm commit their own position.

### The gate

The share of the cursor comparison and the module read at `c2f0341` is 9.182 ns for each iteration, or 5.7 percent of the 161.754 ns loop of that revision. Half of that share is 4.591 ns. The same share at `a0104b4` measures 3.990 ns, or 2.7 percent of the 148.586 ns loop, because #548 removed the pair stack of `Term::eq` from the loop. The task change removes 3.389 ns of that share, which is 85 percent of the share of the current revision and above its half of 1.995 ns. A first session of the same revision, with three runs of the base and three runs of the change at a one-minute load of 6.59 to 11.02, gives a larger fall: base 149.141 ns, change 145.026 ns, or 4.115 ns.

The whole share of the current revision, 3.990 ns, is below the 4.591 ns of half of the historical share, so no change to this loop can reach the historical number at `a0104b4`. The cause is the change of #548, which lowered the whole loop from 161.754 ns to 148.586 ns and lowered the ablation effect of the comparison with it.

### The floor shapes and the added rows before and after

`mise run bench-dispatch` measures the three floor shapes at 20000 and 200000 iterations, and the five added rows at 200000 iterations, on the release build. Six runs of the base `a0104b4` and six runs of the task branch ran interleaved in two sessions at a one-minute load of 2.79 to 11.02. Each cell is the minimum of the six runs of that build. The `after` ratio is the after PBeam number divided by the ERTS number of the same row of the after runs.

| Shape | iterations | before, ns | after, ns | fall, ns | ERTS after, ns | ratio after |
| --- | --- | --- | --- | --- | --- | --- |
| `call_ext_only`, `dispatch_probe` | 20000 | 283.1 | 260.1 | 23.0 | 2.04 | 127.5 |
| `call_ext_only`, `dispatch_probe` | 200000 | 283.3 | 262.3 | 21.0 | 2.37 | 110.7 |
| `call_only`, `dispatch_probe` | 20000 | 146.9 | 143.0 | 3.9 | 2.15 | 66.5 |
| `call_only`, `dispatch_probe` | 200000 | 147.7 | 144.1 | 3.6 | 2.46 | 58.6 |
| `call_only`, position probe | 20000 | 147.6 | 143.5 | 4.1 | 2.16 | 66.4 |
| `call_only`, position probe | 200000 | 147.4 | 143.8 | 3.6 | 2.47 | 58.2 |

The added rows are the added cost of one instruction on the local `call_only` loop at 200000 iterations, which is the probe of the added instruction less the `call_only` floor loop of the same run. The second run of the task branch in the first session is excluded from the subtraction: its `call_only` floor loop measured 215.5 ns and its `call_ext_only` floor loop measured 367.5 ns, which are outliers of that session, and the subtraction of two numbers of that run is not a measurement of one instruction. Each cell is the minimum of the remaining runs.

| Added instruction | before, ns | after, ns | ERTS before, ns | ERTS after, ns |
| --- | --- | --- | --- | --- |
| `gc_bif '+'` | 70.5 | 74.1 | 0.00 | 0.02 |
| the `bif` of `tuple_size/1` | 68.1 | 73.0 | 0.57 | 0.63 |
| the `bif` of `element/2` (`get_tuple_element`) | 73.5 | 76.2 | 0.02 | 0.02 |
| the `bif` of `hd/1` (`get_hd`) | 73.7 | 75.9 | 0.01 | 0.04 |
| the `bif` of `map_get/2` on a 1000-key map | 126.9 | 126.5 | 4.26 | 4.32 |

The change moves no floor shape of the `call_ext_only` kind by less than 21.0 ns and no `call_only` shape by less than 3.6 ns. The added rows move −0.4 ns to +4.9 ns, inside the run-to-run noise of this benchmark.

## 4. What each step decided and what it left open

### Step 0, [#520](https://github.com/fahchen/pbeam/pull/520)

The step decided that the benchmark is a `[[test]]` target that the check gate builds and runs, that the probe holds the three floor shapes, and that the added rows use the local `call_only` loop as their baseline. It decided that step 2's gate of below 400 ns belongs to the position shape, because the local shape was already at 340 ns. It left open the shape of the Terra redraw measurement: the target does not measure the redraw, because the redraw runs only while the application reads keys in raw mode from a pseudo-terminal, and the check gate runs in CI without a terminal. That gap stays open.

### Step 1, [#521](https://github.com/fahchen/pbeam/pull/521)

The step decided that the two tables live in `BeamModule`, that they are `HashMap` values keyed by the label and not a `Vec` indexed by the label, and that they use the FNV-1a mixer. It left open `max_x`, which the issue names, because no consumer exists before step 8, and a field that nothing reads fails the lint gate. It left open the `call_ext_only` regression of 2.6 percent and recorded it as a known gap.

### Step 2, [#523](https://github.com/fahchen/pbeam/pull/523)

The step decided that `Frame::next_instruction` stays the durable resume position and that the loop commits the cursor to it at each call and each exit. It decided that the cursor holds the module and the position only, and not the frame index, because a stored index would need an update at each push, pop, and truncate. It left the removal of the two `String` fields of `Frame` to step 3 and recorded the `u32` wrap of `ModuleId` after 2^32 constructions as a limit that the runtime does not reach.

### Step 3, [#524](https://github.com/fahchen/pbeam/pull/524)

The step decided that a function name is a `NameId`, which is the index of the name in a per-module name table, and that an export key is `(NameId, u32)`. It decided that the code table keeps the loaded modules by `ModuleId` as well as by name. It left open `Instruction::CallExternal`, which still holds the two names of its import, and it named the two allocations that the call path keeps: the argument `Vec<Term>` of the `gc_bif` path, which step 5 owns, and the pair stack of `Term::eq`, which no step of the plan owns. It also left two `String` allocations at `request_grant` and `tail_call_error`, which are outside the measured call path.

### Step 4, [#525](https://github.com/fahchen/pbeam/pull/525)

The step decided that the identity of a BIF is the `BifId` variant of its table entry, that an entry that the table does not implement resolves to `BifId::Unimplemented` and reports a run-time error, and that `Instruction::CallExternal` keeps its two names and adds `bif: Option<BifId>`. It decided that the external call path reaches the export lookup through `call_export` and never through `call_mfa`, after review rejected the first head. It left open the two small classification scans, the dynamic `call_mfa` path, the `function_exported/3` path, and the `ImportId` change of `CallExternal` that the plan names.

### Step 5, [#526](https://github.com/fahchen/pbeam/pull/526)

The step decided that `BifArguments` holds the terms of one `bif` call and hands a borrowed slice to the body, because a slice that borrows the process registers cannot compile beside the `&mut Scheduler` of a BIF body. It decided that only a `bif` instruction takes the borrowed path, because the loader gives that instruction at most three arguments, and that the path follows the shape of the call and not a classification table of BIF identities. It left open `Instruction::CallExternal` of a pure BIF, which still builds one `Vec<Term>`, and it claims no floor-shape gain, because the after runs had a high load and the movement is inside the noise.

### Step 6, [#527](https://github.com/fahchen/pbeam/pull/527)

The step decided to keep the decoded form and to skip step 8. The decision rests on the step 8 gate, under incomplete evidence. It decided to measure the layout and the per-iteration bytes in the library unit tests and the cache proxy in the integration benchmark, because `Instruction` is `pub(crate)` and an integration test cannot name it. It left open the cache-miss count of the instruction slots, which needs a hardware counter, and the Terra redraw measurement on PBeam, which the repository does not have.

### Task #550, the active module and the execution cursor

The task removed the per-iteration comparison of the active module from the run loop. The loop no longer reads the active frame on each iteration, and each exit that can change the active module refreshes the cursor: a call that enters a frame, a return, and an exception that truncates the stack. The refresh compares the integer identity of the two modules, so those paths pay that comparison and no name compare. The `active_position` helper stays for `Cursor::new` only. The test profile asserts that the module of the cursor equals the module of the active frame at the top of each iteration, so a path that changes the active module without a refresh fails a test instead of running another module's code in release.

The task left open two parts of the loop. The per-iteration `scheduler.current_process_mut()` lookup is the process state and not the cursor. The pre-body `cursor.commit` has no measured floor-loop share, and the task keeps it, because the plan commits the position before each body call.

### Step 10, task [#544](https://github.com/fahchen/pbeam/issues/544)

The task decided to keep `Arc`. The floor loop makes 0 `Arc` changes for each iteration, and the estimated share of the counted sites at cache-resident costs is 7.4 percent of the shortest release key press and 2.5 percent of the shortest debug key press, below the 20 percent gate of the plan. The task decided to count with a temporary patch and not with `dtrace` or `lldb`, because the host enables System Integrity Protection, the release build inlines `Arc::clone` and its drop, and a breakpoint at each change stops the process for each hit. It left open the sites that the patch does not count, which section 5 names.

## 5. Known gaps

- `Instruction::CallExternal` of a pure BIF still builds one `Vec<Term>` (`src/runtime/interpreter.rs:3182`, `call_arguments`). The measured set of step 5 is the `bif` instruction, and no step owns this path.
- The two small classification scans stay on the external call path: `PBEAM_BIFS` with 5 entries (`src/runtime/interpreter/bif.rs:977`) and `HEAVY_BIFS` with 7 entries (`src/runtime/interpreter/bif.rs:1029`). The scan of the `call_ext_only` shape is 1.24 ns and the largest single scan is 3.22 ns against a review limit of 5 ns. A `call_ext` of a PBeam BIF runs both scans, and a scratch run measured about 5.4 ns for that path, which is above the limit. That path is not a measured benchmark shape. The trigger to store the classification at load time is a PBeam BIF in a hot path.
- The pair stack of `Term::eq` (`src/term.rs:318`) was one of the two allocations that remained on the call path of step 3, and it held no owning task. [#548](https://github.com/fahchen/pbeam/pull/548) removed the heap allocation of a shallow or a shared comparison, so the floor loop no longer allocates for its `test`. The attribution section above keeps its 8.2 percent number for `c2f0341`, the revision that it measured.
- The L1 data cache residency of step 6 is an assumption. It follows from the measured 816-byte slot range and the measured L1 data cache sizes of the host, and it is not a measurement. The host has no hardware performance counter for cache misses, because macOS has no `perf`, so the cache evidence is a proxy of the host memory system.
- The repository has no Terra benchmark. The 5.0 percent estimate of step 6 is the memory-system difference of the two record strides at the estimated size of one redraw stream, and it is not a measurement of the redraw.
- The run-to-run noise is about 10 percent at 200000 iterations. A single column of a summary table therefore carries that noise, and a change inside it is not a measured effect.
- The end-to-end measurement of section 3 is one run of each mode, and not a minimum of several runs.
- The count of step 10 does not include these `Arc` and `Rc` changes: the `Arc` of the binary sub-views (`src/term/binary.rs:276`, `src/term/binary.rs:431`, `src/term/binary.rs:451`), the child nodes of a map that `Arc::make_mut` copies, the `Arc<str>` of an atom name, the `Arc<AtomTable>`, a direct clone of a `ConsRef`, a `Tuple`, a `Binary`, a `Map`, or a fun that does not go through `Term::clone`, the `Arc::clone` of the match context in `any_match_context` (`src/runtime/interpreter.rs:3604`) and the drop of the returned `Arc<MatchState>`, the `Binary` clone of `MatchState::new(bytes.clone())` (`src/runtime/interpreter.rs:1355`), the release of the child nodes of a map through `Arc::into_inner` in `Map::drain_into` (`src/term/map.rs:320`, `src/term/map.rs:860`), the `Arc::downgrade` of the dirty IO `Claim` (`src/runtime/dirty_io.rs:207`) and the drop of that `Weak` after the call, and the three `Rc<BeamModule>` clones in `mfa_stacktrace` (`src/runtime/interpreter.rs:4655`, one for each stack frame), in `load_nif` (`src/runtime/interpreter/native.rs:227`), and in `error/2` (`src/runtime/interpreter/bif/erlang.rs:516`). The line numbers are at `a4072f1`. Instruction handlers reach the three `Rc` clones: the `error/2` BIF calls `mfa_stacktrace` (`src/runtime/interpreter/bif/erlang.rs:533`), and `CallExternal` reaches `load_nif`. The match-context clone runs at each binary-match instruction: the first key block runs about 940 of them (`BinaryMatch` 212, `BinaryGetPosition` 210, `BinaryGetUtf8` 199, `BinaryMatchStart` 115, `BinarySetPosition` 104, `BinaryGetTail` 97), against 84892 counted `Arc` drops. These sites add an estimated 1 to 2 percent more counts to a key press, which does not change the decision. The trigger to count them is a key-press share of 10 percent or more, half of the gate.
- The cost of one change in step 10 is a micro-measurement with a cache-resident count line. The shares of the Terra key press do not include the cost of a cold count line.

## 6. What comes next

Step 10 ([#518](https://github.com/fahchen/pbeam/issues/518)) has no measured reason to start: task [#544](https://github.com/fahchen/pbeam/issues/544) measured 0 `Arc` changes for each floor-loop iteration and an estimated share of 7.4 percent of a release key press for the counted sites at cache-resident costs, below the 20 percent gate. The owner decides the state of the step.

Step 7 ([#515](https://github.com/fahchen/pbeam/issues/515)) merged as `c6cfb3a` ([#535](https://github.com/fahchen/pbeam/pull/535)) and removed the two measured allocations on the drop path: the placeholder handle of `Map::drain_into` and the unconditional work vector of `Term::drop`. Its acceptance is the drop cost of an immediate, a tuple, a shared map, and a deep list, before and after, with no drop regression and no map operation regression above 2 percent. Its blocked-by link to step 6 is satisfied, because step 6 is merged as `d36f686`.

Step 9 ([#517](https://github.com/fahchen/pbeam/issues/517)) adds combined instructions for the two measured pair shapes, `test` with `gc_bif` and `gc_bif` with `call_only`. Its gate is a floor loop at or below 150 ns for each iteration, and the target for the Terra key latency at that point is at most 10 ms, against the measured 0.018 s to 0.039 s of section 3.

Step 8 ([#516](https://github.com/fahchen/pbeam/issues/516)) is skipped. The gate of that step needs at least a 10 percent gain on the floor loop and at most 32 bytes for each instruction. The step 6 estimate is 5.0 percent under the memory-system proxy, which is below the gate, and the 816-byte slot range of the floor loop is below 1.3 percent of the smaller L1 data cache of the host. The step number does not change, and step 9 follows step 7 without it. Step 9 keeps a blocked-by link to step 8, and that link is stale while step 8 is skipped.

## The `select_val` dispatch decision of task [#545](https://github.com/fahchen/pbeam/issues/545)

This section answers the question of issue [#545](https://github.com/fahchen/pbeam/issues/545): how does the interpreter select the clause of a `select_val` instruction, given the shape of the choice list that real code produces? The measured revision is `915590af`, the head of `origin/main` that the branch of the task starts from, and the environment is the one of "The environment of the numbers" above. The branch holds three later revisions: `b4b1d275` adds the ordered form, `d8ffdb36` adds the compare fast path, and `ea45a0e7` adds the 16-choice probe to the fixture and the harness.

The instruction has two prepared forms, and the decoder of the loader builds one of them one time at load time. A list of at most `SELECT_VALUE_LINEAR_LIMIT` choices keeps the encoded order, and the interpreter walks it with the exact equality of `Term`. The linear walk is the behaviour of every earlier revision. A larger list is sorted by the ERTS term order `Term::compare`, and the interpreter binary-searches it.

The bound is 32, and it is a measured estimate. It is not the bound of the pinned ERTS. ERTS uses 10 in the generator that serves a choice list that a jump table does not fit: that generator special-cases two values with `i_select_val2`, reserves a place for a sentinel and takes `align = 1` for 3 to 10 values, which selects `i_select_val_lins`, and it selects `i_select_val_bins` above that (`erts/emulator/beam/emu/ops.tab:159-169` and `emu/generators.tab:214-254`, OTP 29.1.1). The ordered search of ERTS compares one encoded value at a time, which costs less than the `Term::compare` call of PBeam, so the crossover of ERTS sits below the crossover of PBeam. ERTS sorts the encoded values one time at load time with `beam_load_sort_select_vals` (`erts/emulator/beam/beam_transform_helpers.c:135-146`), so its order of atom choices follows the atom value and not the atom name. PBeam sorts by `Term::compare` and searches with the same order, so the selected clause agrees.

The two end-to-end points of the task bracket the bound: the ordered form loses 13.276 ns for each iteration at 16 choices, net of the 2-choice control, and wins 458.005 ns at 256 choices. The only micro-measurement point between them is 32 choices, where the linear walk costs 152.053 ns in the worst case and the ordered search costs 34.172 ns. No end-to-end probe covers 17 to 32 choices, so 32 is an estimate, and a later session can pin the bound with end-to-end probes at 32 and 64 choices.

### The shape distribution

The command is `escript /tmp/select_val_histogram.escript fixtures <OTP lib dir> <Elixir lib dir>`, run from the repository root on the pinned ERTS. The run reads each BEAM file with `beam_disasm:file/1` and counts the pairs of every `select_val` instruction. The first run is the tree without the fixture of this task: 1777 files and 24568 instructions.

| Choices | Instructions | Share |
| --- | --- | --- |
| 1 to 4 | 20346 | 82.82 percent |
| 5 to 8 | 2055 | 8.36 percent |
| 9 to 16 | 1267 | 5.16 percent |
| 17 to 32 | 527 | 2.15 percent |
| 33 to 64 | 189 | 0.77 percent |
| 65 and more | 184 | 0.75 percent |

The distribution is dominated by small lists: 82.82 percent of the instructions hold at most 4 choices, and 91.18 percent hold at most 8. Only 373 instructions, or 1.52 percent, hold more than 32 choices, so the ordered form serves about one instruction in 66. The largest single list holds 6878 choices.

The fixture of this task, `fixtures/dispatch/select_val_probe.erl`, adds one `select_val` with 256 choices in `bucket/1`, one with 16 choices in `nibble/1`, and one with 2 choices in `digit/1`. The 16-choice probe joined the fixture after the first session of the task, which read 1778 files and 24570 instructions with the two-instruction form of the fixture, of which 20347 are in the 1-to-4 bucket and 185 in the 65-and-more bucket. The fixture covers the 65-and-more bucket, which holds the lists whose linear walk costs the most, and the 9-to-16 bucket, which the bound of 32 keeps on the linear form.

A later run of the same command at `2c667081` on 2026-10-15 on this host reads 1973 BEAM files and 26917 `select_val` instructions: 170 files under `fixtures`, 1356 under the OTP lib directory, and 447 under the Elixir lib directory. Its bucket shares are 83.42, 7.74, 5.49, 1.97, 0.70, and 0.69 percent, which differ from the first table by at most 0.62 percentage points, and 374 instructions, or 1.39 percent, hold more than 32 choices. The record does not identify the cause of the difference in the file count. The record keeps the first table, because the decision of the task used it, and the later run does not change that decision.

### The micro-measurement

The measurement is `select_val_scan_*` and `select_val_binary_search_*` of `tests/runtime/interpreter/data_structure_costs.rs`, inside the test `prints_the_cost_of_every_derivation`. It measures the model of each search arm on an integer list that is already in the term order, with the last choice as the value. The value is the worst case of the linear walk and the `Greater` case of the binary search. Each invocation of the test is the minimum of nine timed runs of 100000 operations. The linear column and the after column come from one release invocation of the test at `d8ffdb36`, after the fast path, and the record of that invocation holds no load. The before column is the minimum of three release invocations in the first session of the task, which measured the code of `b4b1d275` at a one-minute load of 4.39 to 6.04.

| Choices | Linear walk, ns for one operation | Ordered binary search before, ns for one operation | Ordered binary search after, ns for one operation |
| --- | --- | --- | --- |
| 4 | 19.828 | 48.4 | 16.381 |
| 16 | 78.440 | 116.1 | 27.791 |
| 32 | 152.053 | 139.6 | 34.172 |
| 64 | 292.362 | 161.8 | 38.867 |
| 128 | 576.458 | 186.0 | 44.432 |
| 256 | 1145.774 | 207.2 | 50.515 |

The ordered search is now faster than the worst case of the linear walk at every measured shape, and the fast path is the cause. The cause of the before column is `Term::compare`: it built the work vector `vec![Step(self, other)]` for every call (`src/term/order.rs:73`), so one comparison of two integers cost about 25 ns against about 4.5 ns for the exact equality of `Term`. The fast path of `d8ffdb36` compares the pair of the two terms first with no work stack, so a pair that the comparison decides allocates nothing, and only a pair that the comparison does not decide fills the stack: two cons cells, two tuples of equal size, two maps of equal size, and two funs that the own entries do not order.

A branch session re-measures the same test at a one-minute load of 2.10 to 2.88. Its linear rows are 18.111, 70.846, 141.573, 283.228, 564.987, and 1130.336 ns, which agree with the linear column above. Its ordered rows are 9.371, 16.341, 19.827, 23.353, 26.825, and 30.284 ns, which are below the after column above. The direction of the change is firm, and the after column above is conservative.

The linear column is a model of the walk, and the model costs more than the interpreter's own walk: the interpreter reads its encoded choice list in order with the exact equality of `Term`, and the end-to-end session below measures that walk for each iteration.

The issue quotes the linear scan at `912b322` as 85.073 ns, 343.375 ns, and 1417.017 ns for 4, 16, and 64 choices. Those numbers are historical: [#548](https://github.com/fahchen/pbeam/pull/548) removed the pair-stack allocation of `Term::eq` between that revision and this one, so the same model now costs 19.828 ns, 78.440 ns, and 292.362 ns in the table above.

### The end-to-end measurement

The probe is `fixtures/dispatch/select_val_probe.erl`, and the harness is `tests/integration/select_val.rs`. `ordered_loop/2` runs the 256-choice `select_val` of `bucket/1` one time for each iteration, `nibble_loop/2` runs the 16-choice `select_val` of `nibble/1`, and `linear_loop/2` is the control: the same loop shape with a 2-choice `select_val` and with `rem 3` in the place of `band 255`. Each number is the minimum of nine runs of the probe inside one invocation of `cargo test --release --test select_val`, and the number divides the run by the 200000 iterations. The pinned ERTS answers the same loops in 7.72 ns to 8.35 ns for each iteration.

The session interleaves three builds in one worktree. **Before** is the `src/` of `915590af`, the head of `origin/main` that the branch starts from, which keeps the linear walk for every shape. **Middle** is `b4b1d275`, which adds the ordered form with no compare fast path. **After** is `d8ffdb36`, which adds the compare fast path. Every build holds the fixture and the harness of `ea45a0e7`. The `src/` of each build is checked out into the one worktree, so the three builds share the fixture, the harness, the host, and the hour, and a rebuild separates each pair of builds. The task could not run the three revisions as detached checkouts.

| Round | Before `915590af`, 256 | Middle `b4b1d275`, 256 | After `d8ffdb36`, 256 | Before, 16 | Middle, 16 | After, 16 | Before, 2 | Middle, 2 | After, 2 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 992.740 | 672.733 | 526.883 | 506.625 | 584.573 | 517.379 | 488.130 | 489.851 | 487.776 |
| 2 | 990.376 | 672.988 | 528.557 | 507.679 | 582.008 | 518.964 | 489.019 | 491.837 | 491.222 |
| 3 | 984.243 | 684.754 | 526.238 | 500.169 | 583.174 | 514.915 | 486.306 | 488.149 | 489.312 |
| Minimum | 984.243 | 672.733 | 526.238 | 500.169 | 582.008 | 514.915 | 486.306 | 488.149 | 487.776 |

The one-minute load is `{ 4.40 5.95 7.00 }`, `{ 3.71 5.72 6.90 }`, and `{ 3.45 5.60 6.84 }` for the three builds of round 1; `{ 3.28 5.46 6.77 }`, `{ 7.20 6.22 7.02 }`, and `{ 6.16 6.03 6.94 }` for round 2; and `{ 5.53 5.90 6.88 }`, `{ 5.93 5.96 6.88 }`, and `{ 5.17 5.80 6.81 }` for round 3. Every value is below the bound of 12 that this document uses.

The session measures the two forms with the bound of 10, because that was the bound of the task at the time of the run, and the landed bound is 32. The 256-choice rows are direct for the landed change: the shape is above both bounds, so it keeps the ordered form, and it falls from 984.243 ns to 526.238 ns for each iteration, a fall of 458.005 ns or 46.5 percent. The 16-choice rows are historical for the landed change: the bound of 32 keeps the shape on the linear form, so its landed value is the linear arm of the before build, 500.169 ns for each iteration. That arm is the code of this branch, and the run of the final build does not measure 16 choices, so the value is an inference from the same session and not a measurement of the final revision. The ordered rows of the middle and the after build measure the form that the landed change does not use at 16 choices, and they show why the bound moved: the ordered search costs 582.008 ns before the compare fast path of `d8ffdb36` and 514.915 ns after it, against the 500.169 ns of the linear walk. The control of the session moves by at most 1.843 ns, or 0.4 percent, across the three builds.

### The decision

The compare fast path lands. It removes the work-vector allocation of `Term::compare` for a pair that the comparison decides, and the micro-measurement falls the ordered search from 116.1 ns to 27.791 ns at 16 choices and from 207.2 ns to 50.515 ns at 256 choices. The resolved 256-choice shape, which keeps the ordered form under either bound, measures 526.238 ns against the 984.243 ns of the linear form of `915590af`, a fall of 458.005 ns or 46.5 percent.

`SELECT_VALUE_LINEAR_LIMIT` is 32. The bound is a measured estimate between the two end-to-end points of the session: the linear form wins at 16 choices, and the ordered form wins by 458.005 ns at 256 choices. The only micro-measurement point between them is 32 choices, where the linear walk costs 152.053 ns in the worst case and the ordered search costs 34.172 ns; at 16 choices the same worst-case numbers are 78.440 ns and 27.791 ns. Thus the resolved change keeps the linear form at 16 choices, whose value is the 500.169 ns of the same session, and it uses the ordered form above 32 choices.

The pinned ERTS uses 10 (`erts/emulator/beam/emu/generators.tab:240-254`, OTP 29.1.1). Its ordered search compares one encoded value at a time, which costs less than the `Term::compare` call of PBeam, so the crossover of ERTS sits below the crossover of PBeam.

No end-to-end probe covers 17 to 32 choices, so the bound of 32 is an estimate. A later session can pin the bound with end-to-end probes at 32 and 64 choices, and `docs/backlog.md` records that gap with its trigger. The fixture also measures only the 256-choice shape, the 16-choice shape, and the 2-choice control, so no number here covers a `select_val` with 6878 choices.

## The attribution of the Terra key press and of the external call at `b8c71940` (task [#625](https://github.com/fahchen/pbeam/issues/625))

This section answers the question of issue [#625](https://github.com/fahchen/pbeam/issues/625): where does one release Terra Counter key press spend its time, where does one `call_ext_only` floor iteration spend its time, and what causes the +15.5 percent shift of the `call_ext_only` floor loop that starts at `e0d9cbe5`? The measured runtime revision is `b8c71940`, the head of `origin/main` at the time of the sessions. The host is the host of "The environment of the numbers". All sessions ran on 2026-10-15 (JST), with no other `cargo`, `rustc`, or `nextest` process and a one-minute load below 4 in each session that recorded its load. Session S did not record its load (see the table).

### The method

- **The release binary and its line tables.** The time profiles use a second release build of `b8c71940` with line tables only (`CARGO_PROFILE_RELEASE_DEBUG=line-tables-only`, `CARGO_PROFILE_RELEASE_SPLIT_DEBUGINFO=packed`), so that `atos -i` can give each sampled address its inlined functions and its source line. The `__text` section of that build is 1,563,644 bytes, against 1,563,068 bytes for the plain release build (+576 bytes, 0.04 percent). Four interleaved runs of 60 keys (session P) give 27.2 ms and 26.5 ms of CPU time for each key on the plain build, and 27.3 ms and 24.5 ms on the line-table build. These are the raw values of the key window. The difference between the builds (+0.1 ms and −2.0 ms) is inside the spread of one build over its two runs (0.7 ms and 2.8 ms), so the two builds are equal inside the noise of the method. Session R repeats the comparison in two interleaved rounds: 26.7 ms and 26.7 ms on the plain build against 27.0 ms and 27.2 ms on the line-table build.
- **The key driver.** A scratch driver, which follows `scripts/terra_latency.py`, starts the example `terra_counter` on a pseudo-terminal of 80 by 24, waits for the first frame, and sends `j` and `k` in turn every 0.5 s. It records the CPU time of the `pbeam` process with `ps -o time` before and after the keys. Before the keys, it also measures an idle window of 3 s, and it can subtract that idle rate from the key window. The idle window measured no CPU time in each run except `pristine2.log`, which measured one `ps` tick of 10 ms over 3.02 s. The driver extrapolated that one tick over the 30.46 s of the keys and subtracted 1.68 ms for each key (26.5 ms became 24.8 ms). The record does not use that correction: each CPU time here is the raw value of the key window. Each answer is one write of 16 bytes.
- **The time profile.** `xctrace record --template 'Time Profiler' --attach <pid>` samples the process each 1 ms during 100 keys. `xctrace export` gives each sample with its backtrace. A scratch script gives each sample to exactly one part: it expands each frame with `atos -i` and takes the innermost frame that it recognizes, in this order: a frame inside `BeamProgram::run` (`src/runtime/interpreter.rs:658-1987`) is an instruction body, and the arm of `run` that holds the line gives the instruction kind; an allocator frame is an allocation; a `SipHash` or `hashbrown` frame is a hash lookup, and the key type and the first PBeam caller give the map; a `Term` clone or drop frame or an `Arc` count frame is a term clone and drop; a `memcmp` frame is a memory compare. A frame in `src/runtime/interpreter/bif*` marks the BIF path, and a frame of `prim_tty` or a system call marks the IO path.
- **The instruction counts.** The counting probe of task [#544](https://github.com/fahchen/pbeam/issues/544), with one added counter for each module of the active cursor, runs on a scratch copy of `b8c71940`. It reports the counts at each read of the terminal, so one report is the work of one key. The probe is not committed.
- **The time of each module.** A second scratch probe on a copy of `b8c71940` reads the clock (`Instant::now`) when the run loop starts and stops, when the cursor changes its module (`Cursor::new` and `Cursor::refresh`, `src/runtime/interpreter.rs:2791-2818`), and at each read of the terminal. It gives the wall time between two readings to the module of the cursor, so a BIF or a NIF counts for the module that calls it. One key makes 23,734 module changes. The report at each read of the terminal is the time of one key, and the record uses keys 2 to 60 of each run (59 keys), because the count of the first key also differs. The probe is not committed.
- **The floor iteration.** A scratch crate links `pbeam-runtime` of `b8c71940` with line tables and runs `dispatch_probe:floor_ext_loop(200000)` again and again for 8 s under the same Time Profiler. Its ns for each iteration is the 8 s divided by the iterations of the session. The crate checks its deadline only before each run of 200,000 iterations, so the last run can end after the 8 s, and the true mean is up to one run higher: 261.4 ns to 263.1 ns (1/153, or 0.65 percent). Each ns of the floor table can thus be up to 0.65 percent higher.
- **The hardware counters.** `xctrace record --template 'CPU Counters'` records the kept spin probe binaries of `6fd1b3db` and `e0d9cbe5` (the probe of the earlier interleaved session) in four interleaved rounds of 5 s (session C1). Session C2 repeats the four rounds with a rebuild of the same probe that also prints its elapsed time. The command line gives only the guided mode "CPU Bottlenecks". Its export holds the fixed counters (cycles and retired instructions), eight more counters with no name in the export, and four fractions for each 1 ms. Three calibration programs fixed the order of the four fractions: a multiply chain gave 70.7 percent to the second fraction (instruction processing), 4 MB of straight-line code gave 73.6 percent to the third fraction (instruction delivery), and a random branch to two functions gave 65.1 percent to the third and 24.3 percent to the fourth fraction (discarded work). The order is thus useful, processing, delivery, and discarded.

### The sessions

| Session | What | Time | One-minute load, before / after | Raw record |
| --- | --- | --- | --- | --- |
| K1 | Time profile of 100 keys, line-table build | 09:28:39 | 3.12 / 3.80 | `prof2.trace`, `prof2.attr.txt`, `prof2.attr-all.txt` |
| K2 | Time profile of 100 keys, line-table build | 09:48:48 | 1.48 / 1.85 | `prof3.trace`, `prof3.attr.txt`, `prof3.attr-all.txt` |
| P | CPU time for each key, plain and line-table builds, 4 runs of 60 keys, no profiler | 09:46:31 to 09:48:44 | 1.06 to 1.44 | `pristine.log`, `lt.log`, `pristine2.log`, `lt2.log` |
| N | Instruction counts of 10 keys | 09:50:43 | 1.79 / 1.73 | `count2.txt` |
| F1 | Time profile of the floor loop for 8 s, line-table build | 09:40:23 | 1.58 / 1.65 | `floor-tp.trace`, `floor-tp.attr.txt` |
| C1 | CPU Counters, `6fd1b3db` and `e0d9cbe5`, 4 interleaved rounds | 09:36:50 to 09:38:11 | 2.17 to 2.67 | `cc/r<round>-<revision>.trace`, `cc/summary.txt` |
| C2 | CPU Counters, `6fd1b3db` and `e0d9cbe5`, rebuilt probe that prints its elapsed time, 4 interleaved rounds | 11:29:24 to 11:31:04 | 2.12 to 2.69 | `cc2/r<round>-<revision>.out`, `cc2/r<round>-<revision>.trace`, `cc2/r<round>-<revision>.counters.txt`, `cc2/summary.txt` |
| S | `/usr/bin/sample` profiles of the kept spin probe, one run of `6fd1b3db` and one of `e0d9cbe5` | 09:10:05 and 09:10:12 | Not recorded: 1.90 at 09:07, the end of the interleaved session before it, and 3.76 at 09:11:21, the start of the session after it | `interleave/sample-6fd1b3db.txt`, `interleave/sample-e0d9cbe5.txt`, `interleave/spin-run-6fd1b3db.txt`, `interleave/spin-run-e0d9cbe5.txt` |
| L | `bash scripts/terra_latency.sh release` at `b8c71940` | 09:25:42 | 2.05 (before only) | `target/terra-latency-625.txt` of the worktree of this branch |
| R | CPU time for each key of the plain, line-table, and module-time builds, and the time of each module, 2 interleaved rounds of 60 keys, no profiler | 10:17:51 to 10:21:17 | 1.36 to 2.01 | `raw2/r<round>-<build>.log`, `raw2/r<round>-<build>.meta`, `raw2/r<round>-time.time.txt` |

The raw records are in the scratch directory of the session (`/private/tmp/claude-501/-Users-fahchen-PersonalProjects-pbeam/b5d5ab72-9169-48c7-bb96-89c4c337b9e9/scratchpad/625/raw/`; the paths that start with `raw2/` are in `scratchpad/625/raw2/`, and the paths that start with `interleave/` are in `scratchpad/interleave/`), and the repository does not keep them. The harness `bash scripts/terra_latency.sh release` (session L) measured 0.032 s, 0.024 s, 0.030 s, and 0.031 s for the four keys at `b8c71940` at 09:25:42 with a load of 2.05, so the key latency of this revision is in the range of the earlier records.

### One key press

One key press runs 253,558 BEAM instructions. The number is the same for each key after the first key, in session N and in the earlier record of #544. The CPU time of one key is 24.5 ms to 27.8 ms over sessions P, K1, and K2, so one instruction costs about 97 ns to 110 ns on average. The time from the key to the first byte of the answer has a median of 27.1 ms to 30.0 ms.

By instruction count, the four groups of modules sum to the 253,558 instructions of one key (the module counter of the second report of `count2.txt`; the Elixir modules carry the prefix `Elixir.`):

- The Terra render modules run 162,226 instructions (64.0 percent): `Terra.Renderer.Grid`, `Terra.Renderer`, `Terra.Width`, `Terra.Theme`, `Terra.View`, and `Terra.Renderer.Style`.
- The Elixir and OTP library modules run 87,237 instructions (34.4 percent): `Enum`, `Map`, `String`, `unicode`, `unicode_util`, `Keyword`, `Range`, `maps`, `lists`, `String.Chars`, `Access`, `Enumerable.List`, `Enumerable`, and `String.Chars.Integer`.
- The input and output path and the OTP process modules run 3,817 instructions (1.5 percent): `gen_statem`, `group`, `user_drv`, `prim_tty`, `gen_server`, `io_lib_format`, `io`, `Terra.Input`, `gen`, `Terra.Input.Parser`, `io_lib`, `Terra.Terminal.Owner`, `queue`, `file`, `GenServer`, `IO`, `Terra.Terminal.ANSI`, `Terra.Input.TTY`, `Process`, `Terra.Terminal.TTY`, and `Terra.Terminal`.
- The application modules run 278 instructions (0.1 percent): `Terra.Runtime` and `Counter`.

By time, session R gives each group its ms and its share. The probe time is the wall time inside the run loop, and the share is the share of that sum.

| Group of modules | ms for each key | Share | Revision | Session | Load, before / after | Raw record |
| --- | --- | --- | --- | --- | --- | --- |
| Terra render modules | 15.41, 16.38 | 57.1 percent, 56.8 percent | `b8c71940` | R round 1, round 2 | 1.78 / 2.01, 1.60 / 1.40 | `raw2/r1-time.time.txt`, `raw2/r2-time.time.txt` |
| Elixir and OTP library modules | 9.88, 10.57 | 36.6 percent, 36.7 percent | `b8c71940` | R round 1, round 2 | 1.78 / 2.01, 1.60 / 1.40 | `raw2/r1-time.time.txt`, `raw2/r2-time.time.txt` |
| Input and output path and OTP process modules | 1.58, 1.74 | 5.8 percent, 6.0 percent | `b8c71940` | R round 1, round 2 | 1.78 / 2.01, 1.60 / 1.40 | `raw2/r1-time.time.txt`, `raw2/r2-time.time.txt` |
| Application modules | 0.10, 0.13 | 0.4 percent, 0.4 percent | `b8c71940` | R round 1, round 2 | 1.78 / 2.01, 1.60 / 1.40 | `raw2/r1-time.time.txt`, `raw2/r2-time.time.txt` |
| Sum: the wall time inside the run loop | 26.96, 28.82 | 100 percent | `b8c71940` | R round 1, round 2 | 1.78 / 2.01, 1.60 / 1.40 | `raw2/r1-time.time.txt`, `raw2/r2-time.time.txt` |

The sum is 98 percent of the CPU time of the same probe runs (27.5 ms and 29.3 ms for each key), and the probe runs take 0.8 ms and 2.7 ms more CPU time than the plain runs of the session (26.7 ms). Thus the shares are the numbers to use: on a plain key of 26.7 ms, the render modules take about 15.2 ms and the input and output path about 1.6 ms. The render modules run 64.0 percent of the instructions and take about 57 percent of the time. The input and output path runs 1.5 percent of the instructions and takes 5.8 percent to 6.0 percent of the time, which is about 410 ns to 460 ns for each of its instructions, against about 106 ns to 114 ns for the average instruction. The native IO part of the time profile, which is the `prim_tty` NIF and the system calls, is 0.4 percent to 0.8 percent of the time, so most of the input and output path is guest code and the runtime work of its instructions. The time profile does not name the guest module of a sample, so this record does not split the input and output path by mechanism (proposal 7).

The ranked parts of one key press. A part is exclusive: each sample belongs to one part. The ms of a part is its samples divided by the 100 keys of the session, because each sample stands for 1 ms.

| Rank | Part | K1 ms for each key | K1 share | K2 ms for each key | K2 share | Revision |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | Process table lookup: `SipHash` of `ProcessId` in `Scheduler::processes` | 7.40 | 26.5 percent | 6.68 | 26.1 percent | `b8c71940` |
| 2 | Allocation: `malloc`, `free`, and `realloc` | 3.27 | 11.7 percent | 3.05 | 11.9 percent | `b8c71940` |
| 3 | Instruction bodies that rows 6, 7, 9, 10, 11, and 13 do not hold | 2.97 | 10.6 percent | 3.07 | 12.0 percent | `b8c71940` |
| 4 | Term clone and drop: the `Arc` counts and the nested drop | 2.83 | 10.1 percent | 2.42 | 9.5 percent | `b8c71940` |
| 5 | Code table lookup: the module map, the function name table, the export map, the module identity map, and the label map | 1.98 | 7.1 percent | 1.65 | 6.5 percent | `b8c71940` |
| 6 | Instruction body: `Move` | 1.95 | 7.0 percent | 1.61 | 6.3 percent | `b8c71940` |
| 7 | Instruction body: `GetMapElements` | 1.59 | 5.7 percent | 1.32 | 5.2 percent | `b8c71940` |
| 8 | Other: memory copy, other hash maps, and samples whose unwind lost the frame of `run` | 1.51 | 5.4 percent | 1.42 | 5.6 percent | `b8c71940` |
| 9 | Instruction fetch and dispatch: the head of the run loop | 1.33 | 4.8 percent | 1.35 | 5.3 percent | `b8c71940` |
| 10 | Instruction body: `CallExternal`, without its lookups | 1.22 | 4.4 percent | 1.26 | 4.9 percent | `b8c71940` |
| 11 | Instruction body: `Return` | 0.69 | 2.5 percent | 0.45 | 1.8 percent | `b8c71940` |
| 12 | BIF call path and BIF bodies | 0.65 | 2.3 percent | 0.63 | 2.5 percent | `b8c71940` |
| 13 | Instruction body: `CallFun` | 0.43 | 1.5 percent | 0.44 | 1.7 percent | `b8c71940` |
| 14 | IO: the `prim_tty` NIF and the system calls | 0.10 | 0.4 percent | 0.20 | 0.8 percent | `b8c71940` |

K1 has 2,792 samples (27.9 ms for each key, against 27.8 ms of CPU time from `ps`), and K2 has 2,555 samples (25.6 ms, against 25.6 ms). The rows hold every sample: the K1 column sums to 2,792 samples and the K2 column to 2,555 samples (`prof2.attr-all.txt` and `prof3.attr-all.txt`, which print every part). The rank follows K1. The two sessions differ by at most 1.4 percentage points in each row.

The instruction kinds, inclusive of their lookups, allocations, and term work. The executions come from session N, and the time comes from K1.

| Instruction kind | Executions for each key | ms for each key | Share | ns for each execution (computed) |
| --- | --- | --- | --- | --- |
| The head of the run loop, for each instruction | 253,558 | 6.61 | 23.7 percent | 26 |
| `CallExternal` | 11,489 | 4.93 | 17.7 percent | 429 |
| `Move` | 86,934 | 3.93 | 14.1 percent | 45 |
| `GetMapElements` | 10,379 | 2.59 | 9.3 percent | 250 |
| `Return` | 14,600 | 1.43 | 5.1 percent | 98 |
| `CallFun` | 3,555 | 1.40 | 5.0 percent | 394 |
| `Allocate` | 10,718 | 0.69 | 2.5 percent | 64 |
| `PutTuple` | 4,482 | 0.66 | 2.4 percent | 147 |
| `PutMapExact` | 478 | 0.50 | 1.8 percent | 1,046 |
| `Bif` | 3,124 | 0.45 | 1.6 percent | 144 |
| Outside the run loop, or a sample whose unwind lost the frame of `run` | — | 1.11 | 4.0 percent | — |

**The hash lookups, by map and key (K1; K2 agrees inside 1 percentage point).** The counts come from the section "hash lookups: key type x first pbeam caller" of `prof2.attr-all.txt`, and a sum names its lines. The process table `processes: HashMap<ProcessId, ProcessState>` (`src/runtime/scheduler.rs:339`) uses the default `RandomState`, so each lookup hashes the `u64` of the identity with `SipHash`. `Scheduler::current_process_mut` (`src/runtime/scheduler.rs:1420-1425`) gives 546 samples (5.46 ms), `Scheduler::current_module` (`src/runtime/scheduler.rs:1468-1475`) 72 samples, the key compare 67 samples, and `Scheduler::current_process_and_atoms` 22 samples. The head of the run loop calls `current_process_mut` once for each instruction (`src/runtime/interpreter.rs:735`), and the hash lookups of the head are 502 samples (section "arm x category"), or 5.02 ms for each key and about 19.8 ns for each instruction. The call, return, and BIF paths call it again. The release build folds the hash function of `ProcessId` with other functions into one symbol, so a profile without line tables shows it as `<deduplicated_symbol>`. The code table lookups are smaller: the module map by name (`src/runtime/interpreter/code_table.rs:147`) 58 samples (56 with a `str` key and 2 with a `String` key), the module map by `ModuleId` (`code_table.rs:86`, from `Cursor::refresh` and `current_module`) 57 samples (50 lookups and 7 in the hash of the key, `src/beam/module.rs:56`), the export map by `(NameId, arity)` 48 samples (33 at `code_table.rs:228`, 12 at `call_export`, `src/runtime/interpreter.rs:2127`, and 3 in the hash of the key, `src/beam/module.rs:79`), the function name table (`src/beam/module.rs:124`) 24 samples (16 and 8), the atom table 8 samples (5 and 3), and the label map 8 samples (5, 2, and 1).

**The allocations, by instruction kind (K1).** `GetMapElements` 85 samples (3.0 percent; `map_values` collects a `Vec<&Term>` and the arm collects a `Vec<Term>`, `src/runtime/interpreter.rs:1123-1126` and `3166-3176`, and `map_hash` allocates `vec![key]` for each key of a hashmap, `src/term/map.rs:1702`), `Allocate` 50 samples (1.8 percent; each frame grows a new `Vec<StackSlot>`, `src/runtime/interpreter.rs:764-768`), `CallExternal` 42 samples (1.5 percent; the release of the replaced frame and the argument `Vec<Term>` of a BIF), `Move` 33 samples (1.2 percent; the free of the last reference of the overwritten term), `PutTuple` 31 samples (1.1 percent; a tuple is `Arc<Vec<Term>>`, `src/term/tuple.rs:24`, so it takes two allocations), and `CallFun` 22 samples (0.8 percent).

**The term clone and drop, by instruction kind (K1).** `Move` 165 samples (5.9 percent of the key press; the clone of the source in `read_any`, `src/runtime/interpreter.rs:3534-3536`, and the drop of the old value in `write_operand`, `src/runtime/interpreter.rs:4782-4808`), `Deallocate` 20 samples, `CallExternal` 18 samples, `GetTupleElement` 17 samples, and `GetMapElements` 13 samples. The probe of #544 counts 74,381 `Arc` clones and 81,986 drops for each key.

### One `call_ext_only` floor iteration

Session F1 ran 153 runs of 200,000 iterations in 8 s, which is 261.4 ns for each iteration (the best run is 255.1 ns). One iteration retires four instructions: `TestYield`, which the loader writes after the entry label, `test is_eq_exact`, `gc_bif '-'`, and `call_ext_only`. The ns of a part is its share of the 8,051 samples times 261.4 ns, and the rows hold every sample.

| Rank | Part | ns for each iteration | Share | Revision | Session | Load | Raw record |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | Process table lookup: 8 `SipHash` lookups of `ProcessId` | 83.9 | 32.1 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 2 | Code table lookup by name: the module map, the function name table, the export map, the name compares, and the label map | 59.8 | 22.9 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 3 | BIF call path of `gc_bif '-'`: `read_bif_arguments`, `BifArguments::with`, `dispatch_leaf`, and the result | 37.1 | 14.2 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 4 | Instruction body of `CallExternal`, without its lookups: the name classification, the argument check, the frame creation, and `enter_frame` | 36.2 | 13.8 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 5 | Other: samples whose unwind lost the frame of `run`, memory copy, and other hash maps | 22.3 | 8.5 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 6 | Instruction fetch and dispatch: the head of the run loop | 12.8 | 4.9 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 7 | Instruction bodies of `test` and `TestYield` | 8.1 | 3.1 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 8 | Term clone and drop | 0.8 | 0.3 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 9 | Allocation | 0.1 | 0.0 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |
| 10 | IO: system calls outside the run loop | 0.4 | 0.1 percent | `b8c71940` | F1 | 1.58 | `floor-tp.attr.txt` |

**The lookups of one external call, and why each runs.** The iteration makes 12 hash lookups and up to 6 string compares:

- Four process table lookups in the head of the run loop, one for each instruction (`src/runtime/interpreter.rs:735`). The loop cannot keep `&mut ProcessState` across an arm that needs `&mut Scheduler`, so it gets the process again for each instruction.
- One process table lookup in the `Bif` arm of `gc_bif '-'`, which gets the process again after each result that does not trap (`src/runtime/interpreter.rs:999`). F1 gives the hash lookups of the `Bif` arm 271 samples, which is about one lookup: the head of the run loop gives 1,224 samples to its four lookups, or 306 samples for each lookup.
- One process table lookup in `call_external` (`src/runtime/interpreter.rs:1998`), one in `call_export` (`src/runtime/interpreter.rs:2129`), and one in `Cursor::refresh` (`src/runtime/interpreter.rs:2811`). The refresh finds the same module, so it makes no `ModuleId` lookup.
- One `SipHash` lookup of the module name `"dispatch_probe"` in `CodeTable::modules`, a `HashMap<String, Rc<BeamModule>>` (`src/runtime/interpreter/code_table.rs:223` and `147`), with a byte compare of the key.
- One `SipHash` lookup of the function name `"floor_ext_loop"` in the name table of the module, a `HashMap<String, NameId>` (`src/runtime/interpreter/code_table.rs:227`, `src/beam/module.rs:198-200` and `124`), with a byte compare of the key.
- One `SipHash` lookup of `(NameId, arity)` in `BeamModule::exports` (`src/runtime/interpreter/code_table.rs:228`).
- One FNV-1a lookup of the entry label in `function_starts` (`src/runtime/interpreter.rs:3779`, through `frame`, `src/runtime/interpreter/code_table.rs:410`).
- The string compares of the module, the name, and the arity with `("erlang", "apply", 2 | 3)` for a tail call (`src/runtime/interpreter.rs:2000`), with `("erlang", "load_nif", 2)` (`src/runtime/interpreter.rs:2008`), with the 5 entries of `PBEAM_BIFS` (`src/runtime/interpreter.rs:2014`, `src/runtime/interpreter/bif.rs:1107-1108`), and with `("erlang", "apply", 2)` and `("erlang", "apply", 3)` in `call_export` (`src/runtime/interpreter.rs:2122-2123`).

The code table lookups run because `Instruction::CallExternal` still holds the module name and the function name of its import, a decision of step 4 that this record keeps unchanged ("Step 4" in section 4). The approximate split of the 59.8 ns from the key types of F1: the export map about 24 ns, the module map about 13 ns, the function name table about 9 ns, the name compares about 13 ns, and the label map about 3 ns. These sub-numbers come from a second grouping of the same samples, so they overlap and their sum is not the row.

**The pinned ERTS.** ERTS resolves each import to an export entry one time at load time. The loader puts each exported function into the export table (`erts/emulator/beam/jit/asm_load.c:1180-1196`), then gets the export entry of each import with `erts_export_put` and patches the entry into the code (`asm_load.c:1198-1206`; `erts_export_put` is `erts/emulator/beam/export.c:204-217`, and it makes the entry when no module defines the function yet). `call_ext_only` moves the entry into `ARG1` and branches to the address of the active code index in the entry (`emit_i_call_ext_only`, `erts/emulator/beam/jit/arm/instr_call.cpp:157-164`; `emit_setup_dispatchable_call`, `erts/emulator/beam/jit/arm/beam_asm.hpp:388-403`). A later load of the module changes the address in the entry, so the call needs no lookup at run time. ERTS keeps the current process in a register for the whole time slice, so it also makes no process lookup. The pinned ERTS runs the same loop in about 2.4 ns for each iteration ("The floor shapes and the added rows before and after").

### The +15.5 percent shift of `call_ext_only` at `e0d9cbe5`

Session C2 measures the spin probe of the two revisions in four interleaved rounds. Each cell is the median of the four rounds of the revision. The probe of C2 is the probe of C1 with one more output: the elapsed time from the start of the first run to the end of the last run. The mean ns is that elapsed time divided by the iterations. The instructions and cycles for each iteration come from the fixed counters of the covered time: the frequency is the cycles divided by the covered ns (3.20 GHz to 3.22 GHz), the cycles for each iteration are the mean ns times that frequency, and the instructions for each iteration are the instructions for each cycle times the cycles for each iteration. Counter 4 has no name in the export.

| Number | `6fd1b3db` | `e0d9cbe5` | Change | Session | Load | Raw record |
| --- | --- | --- | --- | --- | --- | --- |
| Best ns for each iteration | 231.90 | 257.05 | +25.15 (+10.9 percent) | C2 | 2.12 to 2.69 | `cc2/summary.txt` |
| Mean ns for each iteration (measured elapsed time) | 234.20 | 260.35 | +26.15 (+11.2 percent) | C2 | 2.12 to 2.69 | `cc2/summary.txt` |
| Cycles for each iteration | 752.1 | 836.0 | +84.0 (+11.2 percent) | C2 | 2.12 to 2.69 | `cc2/r<round>-<revision>.counters.txt` |
| Retired instructions for each iteration | 2,949.5 | 2,921.4 | −28.2 (−0.95 percent) | C2 | 2.12 to 2.69 | `cc2/r<round>-<revision>.counters.txt` |
| Instructions for each cycle | 3.92 | 3.50 | −0.43 (−10.9 percent) | C2 | 2.12 to 2.69 | `cc2/r<round>-<revision>.counters.txt` |
| Useful fraction | 52.9 percent | 47.0 percent | −5.9 points | C2 | 2.12 to 2.69 | `cc2/summary.txt` |
| Instruction processing fraction | 43.4 percent | 48.9 percent | +5.5 points | C2 | 2.12 to 2.69 | `cc2/summary.txt` |
| Instruction delivery fraction | 2.15 percent | 2.0 percent | −0.15 points | C2 | 2.12 to 2.69 | `cc2/summary.txt` |
| Discarded fraction | 1.5 percent | 2.0 percent | +0.5 points | C2 | 2.12 to 2.69 | `cc2/summary.txt` |
| Counter 4 for each iteration | 325.2 | 406.5 | +81.3 (+25.0 percent) | C2 | 2.12 to 2.69 | `cc2/r<round>-<revision>.counters.txt` |

Session C1 ran the kept probe binaries of the earlier interleaved session, which print only the best ns and the run count. Its scratch script divided the requested 5 s by the iterations to get the mean ns, but the probe checks its deadline only before each run of 200,000 iterations, so the last run can end after the 5 s. Thus the mean ns of C1 and its cycles and instructions for each iteration are low by up to one run, about 1 percent, and one C1 round gives a mean (257.7 ns) below its best (258.5 ns). The record does not use those three numbers of C1. The ratios of C1 do not use the elapsed time, and they agree with C2: 4.04 against 3.48 instructions for each cycle, and a processing fraction of 41.9 percent against 49.1 percent. The rebuilt binaries of C2 show a smaller shift (+26.15 ns) than the kept binaries of C1 (+33.40 ns in the best ns), and the record does not explain that difference.

The result attributes the shift to the back end of the core, and not to more work. The revision `e0d9cbe5` retires 0.95 percent fewer instructions for each iteration and takes 84 more cycles. Counter 4 equals the processing fraction times the cycles in both revisions (0.434 × 752.1 = 326, and 0.489 × 836.0 = 408), so it counts the cycles of the instruction processing bottleneck, and it grows by 81 of the 84 added cycles. The front end (the delivery fraction falls) and the branch prediction (the discarded fraction grows by 0.5 points, which is about 4 cycles) do not explain the shift. The added cycles are thus stall cycles of the execution back end, such as a load that waits for a store, a load that misses the L1 data cache, or a longer dependency chain.

The `sample` profiles of session S, which ran the same binaries before the counters, spread the added time of +32.4 ns (the best ns of the two runs in `spin-run-6fd1b3db.txt` and `spin-run-e0d9cbe5.txt`, 225.1 ns and 257.4 ns; each function delta is its share times the ns of its run) over many functions: `BeamProgram::run` +9.9 ns, `hash_one::<&str>` +4.2 ns, `Cursor::refresh` +4.0 ns, `read_bif_arguments` +3.4 ns, `call_external` +3.1 ns, `read_test_operand` +2.9 ns, `CodeTable::export_frame` +2.4 ns, `call_export` +2.3 ns, `code_table::frame` +1.9 ns, `Term::eq` +1.5 ns, and `function_at_label` −3.5 ns. The change of the term from 32 bytes to 16 bytes reaches most of these functions, but `hash_one::<&str>` and `Cursor::refresh` read no term, so the record does not name one function as the cause.

The exact event stays unknown. The command line of `xctrace` records only the guided mode, its export does not name counters 2 to 9, and an own selection of the events of the core (for example the L1 data cache misses or the retired mispredicted branches) needs the `kperf` interface, which needs root on this host. The shift is thus attributed to back-end stall cycles with no more retired instructions, and its source line is not identified.

### The proposed tasks

The proposals cover each part at or above 5 percent of the key press or of the floor iteration, in the two ranked tables and in the module table. Each proposal names its parts. Each gate is the same: an interleaved session of the before and the after revision (`call_ext_only`, `call_only`, and the CPU time for each key of the key driver, each with revision, session, load, and raw record), no change of any value (the check gate and the pinned ERTS comparisons pass unchanged), no new `unsafe`, and the accepted `Arc` graph of terms with `Term: Send + Sync`.

1. **Get the current process with no hash (key press 26.1 to 26.5 percent; floor 32.1 percent).** Design: keep the processes in a slab, `Vec<Option<ProcessState>>`, with a free list of slots and a `HashMap<ProcessId, usize>` index for a lookup by identity, and keep the slot of the current process beside `current_process` (`src/runtime/scheduler.rs:339` and `1405-1475`). `current_process_mut` then reads the slot with one bounds check, and only a lookup of another process by identity (a send, a link, a monitor) hashes. A smaller first step is a cheap mixer for the `ProcessId` keys, such as the FNV-1a mixer of `LabelMap` (`src/beam/module.rs:20-46`). Expected gain: about 6 ms for each key and about 70 ns to 80 ns for each floor iteration, if a slot read costs about 1 ns. The measured cost of one lookup is about 10.5 ns in the floor iteration (83.9 ns for 8 lookups) and about 20 ns at the head of the run loop in the key press (19.8 ns for each instruction).
2. **Call an import through a resolved export slot (floor 22.9 percent and a part of the 13.8 percent of the `CallExternal` body; key press 6.5 to 7.1 percent).** Design: the loader gives `Instruction::CallExternal` (`src/beam/code/instruction.rs:388`) an import index into a per-module import table that holds the names, the arity, and the classification that `call_external` now computes with string compares at each call (`erlang:apply/2,3`, `erlang:load_nif/2`, a PBeam BIF, a heavy BIF; `src/runtime/interpreter.rs:2000-2016`, `2122-2123`). The code table of the run keeps a resolved target for each import of each loaded module: the target module handle and its `FunctionStart`, or "not loaded", which goes to `undefined_function` as now. `CodeTable::finish` invalidates the resolved targets when it loads a module, as the ERTS load changes the address of the export entry. The cursor holds the handle of the module and its import slots, so the call reads a slot and makes no hash and no compare (`src/runtime/interpreter.rs:861-878`, `1988-2031`, `2114-2133`; `src/runtime/interpreter/code_table.rs:217-232`). Expected gain: about 50 ns to 60 ns for each floor iteration and about 1.2 ms to 1.4 ms for each key, plus a part of the name compares of the `CallExternal` body. The key-press number is the lookups by name, the label lookups, and the key compares of row 5 (141 samples in K1 and 124 in K2). The `ModuleId` lookups of `Cursor::refresh` and `current_module` (57 samples in K1 and 41 in K2) stay, because the return path keeps them. The gate adds the tests of the error handler, of a module that loads after the first call, and of `erlang:apply/2,3`.
3. **Fetch the map elements in one walk with no allocation (key press: `GetMapElements` 5.2 to 5.7 percent of body, 9.3 percent inclusive).** Design: the arm (`src/runtime/interpreter.rs:1119-1132`) clones the handle of the map one time, so that a destination can be the source register, and writes each value when it finds the key, as `beam_jit_get_map_elements` does (`erts/emulator/beam/jit/beam_jit_common.cpp:706-767`), with no `Vec<&Term>` and no `Vec<Term>`. For a flatmap, the walk goes over the keys of the map one time, because the ERTS loader sorts the pairs of the instruction (`get_map_elements` with `map_key_sort`, `erts/emulator/beam/jit/arm/ops.tab:1190-1197`); the PBeam loader must then keep the pairs in the order of the flatmap keys. For a hashmap, the loader computes the hash of each literal key one time, as ERTS keeps it in the instruction. `map_hash` (`src/term/map.rs:1698-1713`) returns at once for an atom, `[]`, and a small integer, so that it allocates no `vec![key]`. Expected gain: most of the 2.59 ms of the kind, against 250 ns for each execution now.
4. **Remove the avoidable allocations of the instruction path (key press 11.7 to 11.9 percent).** Design, in addition to task 3: keep the Y register storage of a released frame in a small pool of the process and give it to the next `Allocate` (`src/runtime/interpreter.rs:764-768`, `2663-2669`; `Frame::stack`, `src/runtime/process.rs:49-60`). Expected gain: about 3 percent of the key press, measured by the allocation rows of `Allocate`, `Deallocate`, `Return`, `CallLast`, and `CallExternal`. The task does not change the two allocations of a tuple (`Arc<Vec<Term>>`, `src/term/tuple.rs:24`). `Arc<[Term]>` is a rejected design: `Arc::into_inner` needs `T: Sized`, so the drop rule that takes the last tuple out of its `Arc` fails, and a thin header-and-slice `Arc` needs `unsafe` ("Drop rule" in `docs/copy-on-write-terms.md`). `Arc<[Term]>` is also a pointer of 16 bytes, so `Term` grows from 16 bytes to 24 bytes, against the size assertion of `tests/beam/code/instruction.rs:2624`.
5. **Make `Move` cheaper (key press: term clone and drop 9.5 to 10.1 percent, of which `Move` is 5.9 percent; `Move` body 6.3 to 7.0 percent).** Design: first count, with the counting probe, the moves whose source register the next instructions overwrite before they read it. Then the loader marks a move with a dead source, and the arm takes the term out of the source register with no `Arc` increment and no decrement (`src/runtime/interpreter.rs:760-763`, `3534-3536`). The liveness analysis must count the implicit X reads as well as the operands: the arguments of a `func_info` failure (`function_clause`), the argument lists of `undef` and `badarity`, the stack traces, and the start arity of an undefined initial call all read X registers. If a move takes a register that one of these reads later, an error reason changes. ERTS copies one word for a `move` (`emit_i_move`, `erts/emulator/beam/jit/arm/instr_common.cpp:516-519`). Expected gain: up to the 5.9 percent of the clone and drop of `Move` and a part of its 7.0 percent body; the count of the first step sets the number. The gate adds: the liveness counts the implicit X reads, and the error-reason tests pass unchanged. The task does not give the X register file a fixed size at the start to remove the `registers.resize` of each X write (`src/runtime/interpreter.rs:4794`). `registers.len()` has a meaning: it is the start arity of an undefined initial call and its `system_limit` check (`src/runtime/interpreter.rs:667-671`), the length of the release drain (`src/runtime/scheduler.rs:759-764`), and its capacity counts toward the live bytes of the process (`src/runtime/process.rs:633`). A module that loads later can also use a larger X register. A `resize` that does not grow is one compare and one branch, and no profile row measures it.
6. **Run the small-integer `gc_bif` arithmetic in the arm (floor 14.2 percent; key press 2.3 to 2.5 percent).** Design: the `Bif` arm (`src/runtime/interpreter.rs:941-1030`) computes `'-'` and `'+'` of two small integers at once, and every other case, such as a bignum result, takes the present BIF path, as `emit_i_plus` and `emit_i_minus` handle a small result inline (`erts/emulator/beam/jit/arm/instr_arith.cpp:145` and `:334`). Expected gain: most of the 37.1 ns of the floor iteration. This proposal is only for the floor loop, because the key press spends 2.3 percent in the BIF path.
7. **Split the time of the input and output path, then make its largest mechanism cheaper (key press: 5.8 to 6.0 percent of the time, 1.5 percent of the instructions).** Design: first run the module time probe with one more key for each interval inside the modules of the input and output group: the instruction kind, and the BIF or the NIF of a call. The split gives the ms of each mechanism of the 1.6 ms, for example the message send and receive of `gen_statem`, `group`, and `user_drv`, the `prim_tty` NIF and its system calls (0.4 to 0.8 percent in K1 and K2), and the BIF calls. Then the task proposes the change for the largest mechanism, with the ERTS source of that mechanism. Expected gain: not known before the split; the upper bound is the 1.6 ms of the group, and the group costs about 410 ns to 460 ns for each of its instructions against about 106 ns to 114 ns on average. The gate adds the split as the before record of the change.
8. **Dispatch no instruction for `test_heap` and `init_yregs` (key press: the head of the run loop, 4.8 to 5.3 percent).** Design: the loader decodes `test_heap` and `init_yregs` to `Instruction::Noop` (`src/beam/code/instruction.rs:2263-2266`), and the run loop dispatches it with no effect (`src/runtime/interpreter.rs:1913`). One key runs 16,114 `Noop` instructions, 6.4 percent of its 253,558 instructions (`count2.txt`). The loader skips the two opcodes as it skips `line` (`src/beam/code/instruction.rs:2272-2275`), so that no position holds a `Noop`. The rest of the head is the fetch and the jump of each instruction (about 5.2 ns for each instruction), which a `match` interpreter in safe Rust keeps; `TestYield` (22,676 for each key) stays, because it is the reduction check of ERTS (`emit_i_test_yield`, `erts/emulator/beam/jit/arm/instr_common.cpp:3097-3119`). Expected gain: about 0.4 ms for each key before task 1 (16,114 times 5.2 ns of the head and 19.8 ns of its process lookup), and about 0.1 ms after task 1. The gate adds the loader tests that list the decoded instructions.

Three parts at or above 5 percent get no proposal of their own, because each one is a sum of smaller mechanisms:

- Key press row 3, the instruction bodies that the other rows do not hold (10.6 to 12.0 percent). It is the sum of 36 instruction kinds in K1 and 34 in K2. The largest are `Test` (1.3 to 1.7 percent), `TestExactEqual` (1.0 to 1.1 percent), `PutTuple` (1.0 percent), and `PutMapExact` (0.9 percent), so no kind reaches 5 percent.
- Key press row 8 (5.4 to 5.6 percent) and floor row 5 (8.5 percent), the other samples. Their largest parts are the memory copy (1.0 and 0.7 percent of the key press, 1.2 percent of the floor iteration), the other hash maps (0.7 to 1.1 percent and 1.8 percent), and the samples whose unwind lost the frame of `run` (2.4 percent of the floor iteration, `<BeamProgram>` with no line). No part names one mechanism that reaches 5 percent.
- The render modules (56.8 to 57.1 percent of the time) and the library modules (36.6 to 36.7 percent). The module table is a second view of the same time, and the mechanisms of the first table run in each module. Tasks 1 to 6 and 8 thus act on the render and the library time; for example, the head of the run loop and its process lookup are a part of each instruction of each module.

The inclusive table is also a second view of the same samples. Its rows of `Return` (5.1 percent) and `CallFun` (5.0 percent) are about one half hash lookups (68 of 143 and 60 of 140 samples in K1), which task 1 reduces for the process table.

Tasks 1 to 6 follow the order of the measured share, and tasks 7 and 8 cover the two parts near 5 percent. Tasks 1 and 2 also remove or change the functions that hold a net +12.4 ns of the +32.4 ns of the shift of `e0d9cbe5` in the `sample` profiles of session S: `hash_one::<&str>`, `Cursor::refresh`, `call_external`, `CodeTable::export_frame`, `call_export`, `code_table::frame`, `function_at_label`, and `hash_one::<&(NameId, u32)>`.

### Known gaps

- The time profile samples each 1 ms. A leaf function can lose the frame of its caller in the unwind, so 4.0 percent of the key press samples and 5.9 percent of the floor samples do not reach the frame of `run`, and a small number of allocation samples show an implausible caller. Rows 8 and 5 of the two tables hold them.
- The profiles use the line-table build. Its code is 576 bytes longer than the plain release build, and the CPU time for each key of the two builds agrees inside the noise of session P (27.2 ms and 26.5 ms against 27.3 ms and 24.5 ms, raw values), so the record uses the line-table profile for the plain build.
- The CPU time comes from `ps`, which has a resolution of 10 ms; over 60 keys that is about 0.17 ms for each key.
- The key latency is bimodal in each session: the minimum is 12.9 ms to 17.4 ms and the median is 27.1 ms to 30.0 ms. The record does not explain it.
- The time of each module is wall time from a probe build that adds 3 percent to 10 percent of CPU time for each key. A BIF or a NIF counts for the module that calls it, so the library group holds the BIFs that the library calls. The record gives the shares, and it gives the ms on a plain key from those shares.
- The sub-split of the code table lookups of the floor iteration comes from a second grouping of the same samples and overlaps.
- The cause of the shift at `e0d9cbe5` is attributed to back-end stall cycles, and the event and the source line stay unknown, because the counter names and an own event selection are not available without root.
- The counting probe, the module time probe, and the scratch drivers are not committed. The proposals above own any committed measurement; proposal 7 starts from the module time probe.

## The current process with no hash, task [#627](https://github.com/fahchen/pbeam/issues/627)

This section records the gate of issue [#627](https://github.com/fahchen/pbeam/issues/627). The attribution of task [#625](https://github.com/fahchen/pbeam/issues/625) ([PR #626](https://github.com/fahchen/pbeam/pull/626)) measured the `SipHash` lookup of the process table at 26.1 to 26.5 percent of a release Terra key press and at 32.1 percent of a `call_ext_only` floor iteration. The run loop looked up the current process by its pid once for each instruction, and again in the call paths.

### The change

The process table (`src/runtime/scheduler/process_table.rs`) keeps each live process in a slot of a `Vec<Option<ProcessState>>`, with a free list of slots and a `HashMap<ProcessId, usize>` index from the pid to the slot. The table also keeps the slot of the current process in the place of the `current_process` pid of the scheduler. A read of the current process is one bounds check and two `Option` checks, and it does not hash. Only a lookup of a process by its pid, such as a send, a link, or a monitor, hashes the pid. The pinned ERTS keeps the current process as a pointer in the scheduler data (`current_process`, `erts/emulator/beam/erl_process.h:698`, OTP 29.1.1), and the JIT keeps it in a register (`c_p`, `erts/emulator/beam/jit/arm/beam_asm.hpp:86`). The slot takes the place of that pointer.

A process cannot leave the table while it is the current process, and the table asserts it: the scheduler switches the process out before the removal (`Scheduler::finish_current`). Thus a slot that a new process uses again cannot become the current process by mistake. The only iteration of the table is `Scheduler::live_bytes`, which adds the bytes of each process, so the order of the slots is not observable. `erlang:processes/0` and `erts_internal:processes_next/1` stay unsupported (`src/runtime/interpreter/bif/table.rs`), so no BIF exposes an order of the table.

### The session

Two sessions on 2026-10-15 (JST) interleave the before revision `b8c71940`, the head of `origin/main`, and the after revision `ef77e031`, the commit of the change on the branch `perf/627-process-slot`. Each revision has its own release build in its own target directory. The floor numbers come from the test `records_the_interpreter_dispatch_baseline` of `tests/integration/dispatch_baseline.rs` (`--exact --nocapture --test-threads=1`), and each number is the minimum of nine probe runs inside one invocation. The key numbers come from a scratch key driver, which follows `scripts/terra_latency.py`: it starts `PBEAM_BIN=<binary> bash scripts/example.sh terra_counter --no-build` on a pseudo-terminal of 80 by 24, sends 60 keys `j` and `k` in turn every 0.5 s, and reads the CPU time of the `pbeam` process with `ps` before and after the keys. The CPU time for each key is the raw CPU time of the key window divided by 60, with no idle correction. The two revisions run the same compiled example modules. Each session rotates the order of the two revisions in each round, and before each round it waits for a one-minute load below 12 and for no `cargo`, `rustc`, or `nextest` process. No such process ran in any round.

Session s2 measures the Terra key press again, because the key rounds of session s1 did not check the other processes of the host. In s1, the pid of `pbeam` goes from 29300 (round 3, before) to 49589 (round 3, after), so the host started about 20000 processes during round 3. Session s2 also waits for no `erl`, `beam`, or `python` process other than the resident ones, and it records the pid change across each run. It rejects a round and runs it again when a run starts more than 2000 processes or when such a process is present before or after the run. A run without other host work starts 246 to 610 processes. Session s2 rejected one round: its before run started 2539 processes and measured 24.00 ms for each key, below each accepted before run.

| Part | Rounds | Time (JST) | One-minute load | Raw record |
| --- | --- | --- | --- | --- |
| Floor shapes | 8 | 11:55:04 to 11:55:41 | 2.40 to 2.97 | `s1-floor-r<round>-<revision>.txt` |
| Terra keys, s1, replaced by s2 | 3 | 11:55:41 to 11:59:08 | 1.14 to 2.40 before each run | `s1-key-r<round>-<revision>.log` |
| Terra keys, s2, with the host-process check | 3 accepted, 1 rejected | 12:08:34 to 12:13:10 | 1.11 to 1.44 before each run | `s2-key-r<round>-<revision>.log`; the rejected round is in `rejected/` |

The session logs are `s1-session.txt` and `s2-session.txt`. The raw files are in the scratch directory of the task and are not committed.

### The floor shapes

Each value is the minimum over the eight rounds of the session, in ns for each iteration of 200000 iterations. The range is the minimum and the maximum of the eight rounds.

| Shape | Before `b8c71940` | After `ef77e031` | Change |
| --- | --- | --- | --- |
| `call_ext_only` in `dispatch_probe` | 257.1 (257.1 to 260.2) | 149.3 (149.3 to 150.8) | −107.8 ns, −41.9 percent |
| `call_only` in `dispatch_probe` | 102.5 (102.5 to 103.2) | 42.2 (42.2 to 42.4) | −60.3 ns, −58.8 percent |
| `call_only` in `dispatch_position_probe` | 102.5 (102.5 to 103.1) | 42.1 (42.1 to 42.2) | −60.4 ns, −58.9 percent |
| `call_only` plus `gc_bif '+'` | 157.0 | 67.1 | −89.9 ns, −57.3 percent |
| `call_only` plus `tuple_size/1` | 176.9 | 68.8 | −108.1 ns, −61.1 percent |
| `call_only` plus `element/2` | 157.6 | 69.6 | −88.0 ns, −55.8 percent |
| `call_only` plus `hd/1` | 155.0 | 68.9 | −86.1 ns, −55.5 percent |
| `call_only` plus `map_get/2` on a 1000-key map | 189.1 | 101.7 | −87.4 ns, −46.2 percent |

The ranges of the two revisions do not overlap for any shape. Each probe result equals the result of the pinned ERTS in each round, because the test asserts it.

### The Terra key press

The gate value comes from session s2.

| Round of s2 | Before `b8c71940`, ms of CPU for each key | After `ef77e031`, ms of CPU for each key | Median first-byte latency, before / after |
| --- | --- | --- | --- |
| 1 | 26.00 | 17.33 | 27.90 / 18.21 ms |
| 2 | 26.33 | 18.33 | 27.28 / 19.39 ms |
| 3 | 26.33 | 18.33 | 28.41 / 19.43 ms |
| Mean | 26.22 | 18.00 | 27.86 / 19.01 ms |

Each run answered each of the 60 keys. The mean CPU time for each key falls by 8.22 ms, or 31.4 percent. Every after run is below every before run: the highest after run is 18.33 ms, and the lowest before run is 26.00 ms.

Session s1 gave 26.67, 26.83, and 22.33 ms before and 17.83, 17.50, and 18.50 ms after, with means of 25.28 ms and 17.94 ms. Its round 3 ran while the host started about 20000 processes, and its before value of 22.33 ms is the low outlier. Rounds 1 and 2 of s1 (26.67 and 26.83 ms before, 17.83 and 17.50 ms after) agree with s2. The rejected round of s2 shows the same effect: other host work lowers the CPU time of the before revision.

### The decision

The change lands. It removes the hash of the current-process lookup, which was the target of the task, and every measured shape becomes faster by more than the spread of its rounds. The measured gain is larger than the estimate of the #625 proposal (about 70 ns to 80 ns for each `call_ext_only` iteration and about 6 ms for each key). The record does not attribute the part above the estimate.

The cheaper hasher of the issue, alone, was not built: the slot design is small, it needs no `unsafe`, and it removes the hash instead of making it cheaper. The lookup of another process by its pid keeps the standard `SipHash` hasher of `HashMap`; this task does not measure that lookup.

### Known gaps

- The CPU time comes from `ps`, which has a resolution of 10 ms; over 60 keys that is about 0.17 ms for each key. The key latency stays bimodal, as in the record of #625.
- The host-process check of session s2 sees only the start of processes and the `erl`, `beam`, and `python` processes before and after each run. It does not see other work of a resident process during a run.
- The session measures only the floor shapes and the Terra key press. It does not measure a workload with many processes, where the table holds more slots and the lookup by pid is more frequent.
- The free list gives a slot again to a new process, so the table does not shrink after many processes exit. The `Vec` holds one `Option<ProcessState>` for each slot that was ever in use at the same time.

## The resolved export target of an import, task [#628](https://github.com/fahchen/pbeam/issues/628)

This section records the gate of task [#628](https://github.com/fahchen/pbeam/issues/628), proposal 2 of the attribution of task [#625](https://github.com/fahchen/pbeam/issues/625). Before the change, each `call_ext` of module code compared its module, function, and arity with `erlang:apply/2,3`, `erlang:load_nif/2`, and the two small BIF tables, and then looked up the module by name, the function name, the export key, and the entry label. ERTS resolves each import to an export entry one time at load time and branches through the entry (`erts/emulator/beam/jit/asm_load.c:1198-1206`, `erts/emulator/beam/export.c:204-217`, `erts/emulator/beam/jit/arm/instr_call.cpp:150-164`, OTP 29.1.1).

The change has two commits. `6595dcc8` keeps the `ImpT` entries of a module in `BeamModule::imports`, with the call that the loader decides for each entry, and `Instruction::CallExternal` holds the import index. `8e1b44b4` gives each import of each loaded module of a run a slot in the code table. The first call that finds the export keeps its target in the slot, and each later call reads the slot and makes no lookup by name. A call of a BIF with a `BifId` compares no name; a call of a PBeam BIF has no `BifId` and still matches its name in `named_bif`. A call that finds no export keeps the slot empty, so the next call finds a module that loaded after it. The fast path also reuses the process of the call and makes no second process table lookup.

### The session

The before revision is `b8c71940`, the head of `origin/main` that the branch starts from. The after revision is `8e1b44b4`. Each revision is a `git archive` of its commit, built with `cargo build --locked --release --bin pbeam` and `cargo test --locked --release --test dispatch_baseline --no-run` in its own target directory. The session ran on 2026-10-15 from 12:51:44 to 12:55:58 JST on the host of "The environment of the numbers", with no other `cargo`, `rustc`, or `nextest` process. The floor rows run `records_the_interpreter_dispatch_baseline --exact --nocapture --test-threads=1`, eight rounds in rotated order, and each value is the minimum of nine probe runs inside one invocation. The Terra rows send 60 `j` and `k` keys at a gap of 0.5 s to the release Terra Counter (`PBEAM_BIN=<bin> bash scripts/example.sh terra_counter --no-build`). The one-minute load is 2.03 to 2.48 for the floor rounds and 1.36 to 2.22 for the three Terra rounds of this session. A second interleaved session on 2026-10-15 from 13:06:37 to 13:15:49 JST ran eight Terra rounds of each revision in rotated order with the same two binaries, at a one-minute load of 1.09 to 1.97. Each Terra value is the raw CPU time of the `pbeam` process over the key window divided by the answered keys, with no idle correction (see "The idle correction of the Terra rows").

| Shape | Before `b8c71940`, minimum of 8 rounds | After `8e1b44b4`, minimum of 8 rounds | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, 200000 iterations | 257.0 ns (rounds 257.0 to 258.9) | 122.5 ns (rounds 122.5 to 126.0) | -134.5 ns, -52.3 percent |
| `call_only` floor loop, 200000 iterations | 102.0 ns (102.0 to 103.0) | 102.1 ns (102.1 to 107.9) | +0.1 percent |
| Position `call_only` floor loop, 200000 iterations | 101.8 ns (101.8 to 103.0) | 102.0 ns (102.0 to 103.9) | +0.2 percent |
| `call_only` plus `gc_bif '+'`, 200000 iterations | 156.7 ns | 157.5 ns | +0.5 percent |
| `call_only` plus the `bif` of `tuple_size/1` | 177.1 ns | 173.6 ns | -2.0 percent |
| `call_only` plus the `bif` of `element/2` | 157.0 ns | 157.4 ns | +0.3 percent |
| `call_only` plus the `bif` of `hd/1` | 154.1 ns | 155.8 ns | +1.1 percent |
| `call_only` plus the `bif` of `map_get/2` on a 1000-key map | 189.4 ns | 187.9 ns | -0.8 percent |

| Terra round | Before `b8c71940`, ms of CPU time for each key | After `8e1b44b4`, ms of CPU time for each key |
| --- | --- | --- |
| Second session, round 1 | 27.000 | 21.500 |
| Second session, round 2 | 24.167 | 24.667 |
| Second session, round 3 | 27.000 | 24.667 |
| Second session, round 4 | 26.000 | 23.667 |
| Second session, round 5 | 26.000 | 24.333 |
| Second session, round 6 | 22.667 | 25.167 |
| Second session, round 7 | 27.333 | 25.000 |
| Second session, round 8 | 27.000 | 24.500 |
| Second session, mean of 8 rounds | 25.896 | 24.188 |
| Second session, median of 8 rounds | 26.500 | 24.583 |
| First session, rounds 1 to 3 | 27.500, 27.000, 26.000 | 22.500, 25.167, 25.167 |
| First session, mean of 3 rounds | 26.833 | 24.278 |

The `call_ext_only` shape falls by 134.5 ns for each iteration, and the two ranges of the eight rounds do not overlap. The attribution of task #625 expected about 50 ns to 60 ns from the lookups by name, plus a part of the 13.8 percent of the `CallExternal` body. The change also removes the name compares, the classification scans, and the second process table lookup of the call. This record does not split the 134.5 ns between those parts. The shapes with no external call move by -2.0 to +1.1 percent at the minimum, and some move outside the spread of the rounds in both directions. The ranges of the eight rounds do not overlap for `hd/1` (154.1 to 155.1 ns before, 155.8 to 156.9 ns after, +1.1 percent) and for `tuple_size/1` (177.1 to 178.6 ns before, 173.6 to 176.5 ns after, -2.0 percent), and for `map_get/2` they touch only at 189.4 ns. The `hd/1` shape is a measured regression of 1.7 ns. The change does not touch the `bif` instruction, so the cause is probably code layout. The medians of the eight rounds also move: `call_only` from 102.35 ns to 103.10 ns and position `call_only` from 102.25 ns to 102.95 ns, each +0.7 percent, which the minimums hide.

In the second session the CPU time for each key falls by 1.708 ms, or 6.6 percent, at the mean of the eight rounds, and by 1.917 ms, or 7.2 percent, at the median. The ranges of the two revisions overlap, and the after round is below the before round in six of the eight rounds. The expected gain of the attribution was 1.2 ms to 1.4 ms for each key. The resolution of the CPU time of `ps` is 10 ms, or about 0.17 ms for each key over 60 keys, so the Terra number is weaker than the floor number.

The raw records are the session logs `session.txt` and `session-terra2.txt` and the files `floor-r<round>-<revision>.txt`, `terra-r<round>-<revision>.log`, and `terra2-r<round>-<revision>.log` of the two sessions, which stay outside the repository.

### The idle correction of the Terra rows

The profile script subtracts the CPU rate of a 3 s idle window from the key window. The idle window holds 0 or 1 tick of 10 ms of `ps`, so one tick changes the result for each key by about 1.7 ms. In the first session one tick in the idle window of round 2 of `8e1b44b4` (`idle cpu 0.0100 s over 3.02 s`) moved that round from 25.167 ms to 23.488 ms, and the reported after mean from 24.278 ms to 23.718 ms. Round 4 of `8e1b44b4` of the second session had the same tick, which moved it from 23.667 ms to 21.993 ms. The record thus uses the raw CPU time with no idle correction for each Terra row.

### Known gaps

- The Terra ranges of the two revisions overlap in the second session (22.667 to 27.333 ms before, 21.500 to 25.167 ms after), so the 1.708 ms fall has a weak bound.
- The `hd/1` shape is 1.1 percent slower, and the record does not show the cause.
- The record does not split the fall of the `call_ext_only` shape by part.
- The `ModuleId` lookup of `Cursor::refresh` for a call into another module stays, as the attribution of task #625 expected.

## The Y register storage of a released frame, task [#630](https://github.com/fahchen/pbeam/issues/630)

This section records the gate of task [#630](https://github.com/fahchen/pbeam/issues/630), a proposal of the attribution of task [#625](https://github.com/fahchen/pbeam/issues/625). Each body call gives its frame a new slot array at `allocate`, and the return frees it. ERTS keeps the Y registers on the process stack, with no separate slot array. `allocate` first checks the free space of the heap and the stack, which can start a garbage collection (`emit_gc_test`, `erts/emulator/beam/jit/arm/instr_common.cpp:137-155` and `:211` at `OTP-29.1.1`). Then it reserves the Y registers by moving the stack pointer `E`, and `deallocate` only moves `E` back (`emit_allocate_heap` and `emit_deallocate`, `instr_common.cpp:206-229`). The proposal expected about 3 percent of the CPU time for each release Terra key press.

The measured change is commit `1c344478` of the branch `perf/630-frame-pool`. A process keeps up to 8 empty slot arrays of at most 32 slots in a free list. A return and a tail call put the slot array of the released frame in the list, and the `allocate` of a frame with no slot array takes one from it. The live bytes of the process count the list. The change keeps the `Arc` graph of terms and changes no value. Commit `00e3f44e` reverts it, because the gate below shows no gain on the shape that the change targets.

### The session

The before revision is `e93523c3`, the head of `origin/main` that the branch starts from. The after revision is `1c344478`. Each revision is a `git archive` of its commit, built with `cargo build --locked --release --bin pbeam` and `cargo test --locked --release --test dispatch_baseline --no-run` in its own target directory. One interleaved session ran on 2026-10-15 from 17:20:30 to 17:27:45 JST on an Apple M1 Max with 10 CPUs: eight rounds of `records_the_interpreter_dispatch_baseline --exact --nocapture --test-threads=1`, then six rounds of 60 keys of `terra_counter` with the release `pbeam`, each round in rotated order. The one-minute load is 1.02 to 2.93, and no other `cargo`, `rustc`, or `nextest` process ran. Each floor round is the minimum of nine probe runs inside one invocation.

| Shape | Before `e93523c3`, minimum of 8 rounds | After `1c344478`, minimum of 8 rounds | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, 200000 iterations | 47.7 ns (rounds 47.7 to 56.8) | 49.1 ns (49.1 to 49.4) | +1.4 ns, +2.9 percent |
| `call_only` floor loop, 200000 iterations | 42.4 ns (42.4 to 43.6) | 44.2 ns (44.2 to 44.5) | +1.8 ns, +4.2 percent |
| Position `call_only` floor loop, 200000 iterations | 42.4 ns (42.4 to 43.6) | 44.2 ns (44.2 to 44.5) | +1.8 ns, +4.2 percent |

| Terra round | Before `e93523c3`, ms of CPU time for each key | After `1c344478`, ms of CPU time for each key |
| --- | --- | --- |
| Round 1 | 16.000 | 15.500 |
| Round 2 | 16.167 | 14.000 |
| Round 3 | 14.167 | 15.833 |
| Round 4 | 14.667 | 16.000 |
| Round 5 | 16.500 | 15.667 |
| Round 6 | 16.500 | 15.833 |
| Mean of 6 rounds | 15.667 | 15.472 |
| Median of 6 rounds | 16.083 | 15.750 |

The Terra rows are the raw CPU time of `pbeam` for each key, with no idle correction. The idle correction changes only round 1 of `e93523c3`, to 14.319 ms. One tick of 10 ms of `ps` is 0.167 ms for each key.

A second interleaved session on 2026-10-15 from 17:30:55 to 17:31:07 JST runs a body-call probe with the same two binaries: `loop(N) -> _ = f(N), loop(N - 1).`, `f(N) -> _ = g(N), N.`, and `g(N) -> {N}.`, 3000000 iterations, each with two `allocate` instructions, one return of a frame with Y registers, and one `call_last`. It has eight rounds in rotated order, and the one-minute load is 3.50 to 3.83. The user CPU time of the run is 0.75 s to 0.78 s before (median 0.76 s) and 0.58 s to 0.59 s after (median 0.59 s), which is 22 percent less, or about 57 ns for each iteration.

### The decision

The change does not land. The CPU time for each release Terra key press, which is the shape that the change targets, falls by 0.195 ms (1.2 percent) at the mean and by 0.333 ms (2.1 percent) at the median. That is one or two ticks of `ps`, the ranges of the two revisions overlap (14.167 to 16.500 ms before, 14.000 to 16.000 ms after), and the after round is below the before round in four of six rounds. The expected 3 percent is about 0.47 ms, and the session does not show it. The three floor loops, which have no `allocate`, are slower by 1.4 ns to 1.8 ns, and the ranges of the `call_only` rows do not overlap.

The body-call probe shows that the free list removes most of the cost of a frame allocation. But the Terra key press spends too little of its time in frame allocations for the gain to show. The 11.7 percent of allocation in the attribution of task #625 thus includes more than the slot arrays of frames.

In `1c344478`, `call_only`, `call_last`, and the tail call of `enter_frame` call `replace_frame`, which puts the slot array of the released frame in the free list. Thus the code of the free list runs in each iteration of the three floor loops.

Two variants of the working tree tried to remove the floor cost. Their probes ran on 2026-10-15 from 17:28:25 to 17:29:35 JST, outside the measurement rule: no load is recorded, and each variant has only three runs. Each run pairs the variant with a run of `e93523c3`, and the numbers are the minimum of the `call_only` floor loop of 200000 iterations. A variant that keeps the slot array in the frame at a tail call, with `#[inline]` on the three new methods, measured 45.7 ns (`p4-wip.txt` to `p6-wip.txt`), against 42.4 ns of `e93523c3` (`p4-e93523c3.txt` to `p6-e93523c3.txt`). A variant that changes only `allocate` and the return measured 45.4 ns (`p7-wipc.txt` to `p9-wipc.txt`), against 42.5 ns of `e93523c3` (`p7-e93523c3.txt` to `p9-e93523c3.txt`). In this variant, the floor loops run the interpreter source of `e93523c3`, but `ProcessState` has the new field `free_stacks`. The files `p1-wip.txt` to `p3-wip.txt` measure an earlier state of the working tree that the record does not keep, and the record does not use them. The floor cost stays when the floor loop runs the interpreter source of `e93523c3`. The cause, the code layout of the run loop or the layout of `ProcessState`, is not found.

The raw records are the session logs `session.txt` and `alloc-session.txt` and the files `floor-r<round>-<revision>.txt`, `terra-r<round>-<revision>.log`, and `alloc-r<round>-<revision>.txt` of the two sessions, and the files `p<run>-<variant>.txt` of the variant probes, which stay outside the repository.

### Known gaps

- The session does not resolve a change of less than about 0.5 ms for each key on the Terra key press.
- The record does not find the cause of the floor cost: the code layout of the run loop or the layout of `ProcessState`.
- The record does not split the 11.7 percent of allocation of a Terra key press by allocation site.

## The map elements in one walk, task [#629](https://github.com/fahchen/pbeam/issues/629)

This section records the gate of task [#629](https://github.com/fahchen/pbeam/issues/629), from the attribution of task [#625](https://github.com/fahchen/pbeam/issues/625). That attribution measured `GetMapElements` at 5.2 to 5.7 percent of a release Terra key press as body and at 9.3 percent inclusive. The arm collected the values in a `Vec<&Term>`, copied them into a `Vec<Term>`, and searched a flatmap once for each key. `map_hash` allocated a vector also for an atom, `[]`, or a small integer key.

### The change

The change has two commits. `3c83cf88` lets `map_hash` start its walk only for a list or a tuple key, so an atom, `[]`, a small integer, or a binary key allocates nothing. `028221a9` sorts two or more pairs of `get_map_elements` in flatmap key order in the loader, as the ERTS loader does (`beam_load_map_key_sort`, `beam_transform_helpers.c:53-115`; JIT `jit/arm/ops.tab:1190-1197`). The arm keeps one copy of the map handle, walks a flatmap once, and writes each value when it finds the key, as `beam_jit_get_map_elements` does (`beam_jit_common.cpp:706-767`). A hashmap finds each key in turn.

### The session

The before revision is `e93523c3`, the head of `origin/main` that the branch starts from. The after revision is `028221a9`. Each revision is a `git archive` of its commit, built with `cargo build --locked --release --bin pbeam` and `cargo test --locked --release --test dispatch_baseline --no-run` in its own target directory. One session on 2026-10-15 (JST) runs the two revisions in rotated order in each round. The floor numbers come from the test `records_the_interpreter_dispatch_baseline` (`--exact --nocapture --test-threads=1`), and each number is the minimum of nine probe runs inside one invocation. The map shapes come from a scratch module `gme_probe`, compiled with the pinned `erlc` and run with `/usr/bin/time -p pbeam gme_probe <shape> -- gme_probe.beam`. Each run makes 3000000 iterations of one `get_map_elements` of three atom keys: `flat` reads a flatmap of 6 atom keys, and `hash` reads a hashmap of 43 keys. The value is the user CPU time of the process. The Terra numbers come from the scratch key driver of the #628 record: 60 keys to the release `terra_counter` example, with the raw CPU time of `pbeam` from `ps` and no idle correction. A second session of eight Terra rounds follows the first session.

| Part | Rounds | Time (JST) | One-minute load | Raw record |
| --- | --- | --- | --- | --- |
| Floor shapes | 6 | 16:19:36 to 16:19:50 | 2.41 to 2.53 | `floor-r<round>-<revision>.txt` |
| Map shapes | 6 | 16:19:51 to 16:20:09 | 2.30 to 2.64 | `micro-<shape>-r<round>-<revision>.txt` |
| Terra keys, first session | 4 | 16:20:09 to 16:24:45 | 1.36 to 2.51 before each run | `terra-r<round>-<revision>.log` |
| Terra keys, second session | 8 | 16:25:04 to 16:34:15 | 1.06 to 2.49 before each run | `terra2-r<round>-<revision>.log` |

No `cargo`, `rustc`, or `nextest` process ran during the sessions. The session logs are `session.txt` and `session-terra2.txt`. The raw files are in the scratch directory of the task and are not committed.

### The results

Each floor value is the minimum over the six rounds, in ns for each iteration of 200000 iterations, with the range of the six rounds. Each map value is the minimum over the six rounds, in s of user CPU time for 3000000 iterations, with the range.

| Shape | Before `e93523c3` | After `028221a9` | Change |
| --- | --- | --- | --- |
| `flat` map shape, three keys of a 6-key flatmap | 0.75 s (0.75 to 0.76) | 0.53 s (0.53 to 0.54) | -29.3 percent |
| `hash` map shape, three keys of a 43-key hashmap | 0.92 s (0.92 to 0.94) | 0.59 s (0.59 to 0.60) | -35.9 percent |
| `call_only` plus the `bif` of `map_get/2` on a 1000-key map | 102.4 ns (102.4 to 105.6) | 88.1 ns (88.1 to 88.4) | -14.0 percent |
| `call_ext_only` floor loop | 47.7 ns (47.7 to 47.8) | 47.6 ns (47.6 to 47.7) | -0.2 percent |
| `call_only` floor loop | 42.4 ns (42.4 to 43.6) | 42.1 ns (42.1 to 43.0) | -0.7 percent |
| Position `call_only` floor loop | 42.4 ns (42.4 to 43.8) | 42.1 ns (42.1 to 42.9) | -0.7 percent |
| `call_only` plus `gc_bif '+'` | 67.3 ns (67.3 to 67.8) | 66.9 ns (66.9 to 67.2) | -0.6 percent |
| `call_only` plus the `bif` of `tuple_size/1` | 69.3 ns (69.3 to 69.4) | 68.3 ns (68.3 to 69.7) | -1.4 percent |
| `call_only` plus the `bif` of `element/2` | 70.0 ns (70.0 to 70.6) | 69.6 ns (69.6 to 69.8) | -0.6 percent |
| `call_only` plus the `bif` of `hd/1` | 69.3 ns (69.3 to 69.4) | 68.8 ns (68.8 to 69.3) | -0.7 percent |

| Terra round | Before `e93523c3`, ms of CPU time for each key | After `028221a9`, ms of CPU time for each key |
| --- | --- | --- |
| Second session, rounds 1 to 8 | 14.333, 15.667, 16.333, 16.500, 16.333, 16.000, 16.000, 16.000 | 15.833, 15.000, 12.667, 16.333, 15.333, 15.333, 16.167, 13.667 |
| Second session, mean of 8 rounds | 15.896 | 15.042 |
| Second session, median of 8 rounds | 16.000 | 15.333 |
| First session, rounds 1 to 4 | 16.167, 14.833, 16.500, 15.667 | 16.167, 15.667, 13.000, 15.667 |
| First session, mean of 4 rounds | 15.792 | 15.125 |

Each run answered each of the 60 keys. The two map shapes fall by about 73 ns and 110 ns for each iteration, and the ranges of the two revisions do not overlap. The `map_get/2` shape on a 1000-key hashmap falls by 14.3 ns, because its key hash no longer allocates. The other floor shapes do not change beyond the spread of their rounds.

In the second session the CPU time for each key falls by 0.854 ms, or 5.4 percent, at the mean, and by 0.667 ms, or 4.2 percent, at the median. The after round is below the before round in six of the eight rounds. The ranges of the two revisions overlap (14.333 to 16.500 ms before, 12.667 to 16.333 ms after), so this session does not resolve the Terra change: a Welch t test gives t = 1.68 and p = 0.12, and a paired t test of the rounds gives t = 1.54 and p = 0.17. The first session agrees: -0.667 ms, or -4.2 percent, at the mean of four rounds.

### The decision

The change lands. Both map shapes and the `map_get/2` shape fall by more than the spread of their rounds, and no floor shape becomes slower. The Terra mean falls in both sessions, but the fall is not significant. The ERTS behavior stays: the loader refuses a register key among other keys, as the ERTS loader does, and an empty key list matches each map, as the inline JIT path does (`jit/arm/instr_map.cpp:404-415, 464`).

### Known gaps

- The Terra ranges of the two revisions overlap, and the 0.854 ms fall is not significant (p = 0.12). The CPU time from `ps` has a resolution of 10 ms, about 0.17 ms for each key over 60 keys.
- The sessions do not check the other processes of the host, as session s2 of task #627 did. They check only the load and the absence of `cargo`, `rustc`, and `nextest`.
- The map shapes use the user CPU time of `/usr/bin/time`, with a resolution of 10 ms over about 0.5 s to 0.9 s.
- `has_map_fields` keeps its own search of each key, because the task covers only `get_map_elements`.
- The loader keeps each key as an `Operand`, so the arm reads each key through `read_ref` for each comparison. ERTS also stores the hash of each key in the instruction; PBeam computes the hash of each key of a hashmap at each run.

## The source term of a move whose source is dead, task [#631](https://github.com/fahchen/pbeam/issues/631)

This section records the gate of task [#631](https://github.com/fahchen/pbeam/issues/631), proposal 5 of the attribution of task [#625](https://github.com/fahchen/pbeam/issues/625). A `move` from a register copies its term, so a move of a boxed term increments a reference count, and the later write of the dead source decrements it. ERTS copies one word for a `move` (`emit_i_move`, `erts/emulator/beam/jit/arm/instr_common.cpp:516-519`, OTP 29.1.1). The change does not land.

### The change

Commit `4fc08860` adds `Instruction::MoveTake`. The loader marks a `move` from an X or Y register when no later instruction on its straight path reads the register before a write that ends it. A call ends an X register at or above its arity, and `return` ends each X register except `x0`, as the BEAM validator keeps only `x0` after a body call and no register after a tail call or `return` (`schedule_out(0, ...)`, `verify_return`, and `kill_state`, `lib/compiler/src/beam_validator.erl:1204-1257` and `1584-1586`). `deallocate` and a tail call end a Y register, and `trim` moves it to a lower index. The analysis stops with "live" at each instruction that can branch, raise, or read registers that are not its operands, so the implicit reads keep their registers: the arguments of a `func_info` failure, the argument lists of `undef` and `badarity`, the stack traces, the `catch` handlers, and the start arity of an undefined initial call. A non-tail call stops the analysis of a Y register, because a `catch` handler of the same frame can read it after an exception. The interpreter takes the term out of a marked source with no reference count change. `mise run check-brief` passed with 16978 of 16978 application lines, and the error-reason tests and the pinned ERTS comparisons passed unchanged.

### The count

A scratch probe build of `4fc08860` counted the moves from a register on 60 Terra key presses (59 key intervals). For each key press:

| Move | Count | Boxed term |
| --- | --- | --- |
| Live source, a copy | 37,124 | 19,363 |
| Dead X source, a take | 26,428 | 18,173 |
| Dead Y source, a take | 15,567 | 9,299 |

Of the 79,119 moves from a register, 41,995 (53 percent) have a dead source. Of the 46,835 moves of a boxed term, 27,472 (59 percent) are takes, so the change removes 27,472 increments and 27,472 decrements for each key press. The count supported a prototype, so the gate measured it.

### The session

The before revision is `e93523c3`, the head of `origin/main` that the branch starts from. The after revision is `4fc08860`. Each revision is a `git archive` of its commit, built with `cargo build --release --locked` and `cargo test --release --locked --test dispatch_baseline --no-run` in its own target directory. The first session ran on 2026-10-15 from 18:21:44 to 18:25:28 JST on the host of the earlier records (Apple M1 Max, 10 CPUs). The floor rows are 6 rounds in rotated order, each the minimum of nine probe runs inside one invocation, with a one-minute load of 3.74 to 4.15. The Terra rows are 3 rounds of 60 keys in rotated order, with a one-minute load of 2.54 to 3.76. A second session of 8 Terra rounds ran from 18:25:34 to 18:43:07 JST with the same binaries and a one-minute load of 2.50 to 7.34. It waited from 18:26:43 to 18:35:39 for builds of other work to stop. No `cargo`, `rustc`, or `nextest` process ran during a measured run. The Terra rows are the raw CPU time of `pbeam` with no idle correction; each idle window held 0 ticks.

| Shape | Before `e93523c3`, minimum of 6 rounds | After `4fc08860`, minimum of 6 rounds | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, 200000 iterations | 47.7 ns (rounds 47.7 to 47.8) | 47.5 ns (47.5 to 47.7) | -0.4 percent |
| `call_only` floor loop, 200000 iterations | 42.3 ns (42.3 to 43.1) | 41.8 ns (41.8 to 42.5) | -1.2 percent |
| Position `call_only` floor loop, 200000 iterations | 42.3 ns (42.3 to 43.4) | 41.7 ns (41.7 to 42.5) | -1.4 percent |
| `call_only` plus `gc_bif '+'` | 67.2 ns | 66.6 ns | -0.9 percent |
| `call_only` plus the `bif` of `tuple_size/1` | 69.2 ns | 68.6 ns | -0.9 percent |
| `call_only` plus the `bif` of `element/2` | 69.8 ns | 69.3 ns | -0.7 percent |
| `call_only` plus the `bif` of `hd/1` | 69.1 ns | 68.5 ns | -0.9 percent |
| `call_only` plus the `bif` of `map_get/2` on a 1000-key map | 105.4 ns (105.4 to 105.6) | 101.6 ns (101.6 to 103.6) | -3.6 percent |

| Terra round | Before `e93523c3`, ms of CPU time for each key | After `4fc08860`, ms of CPU time for each key |
| --- | --- | --- |
| First session, rounds 1 to 3 | 14.167, 15.667, 16.500 | 16.500, 14.000, 14.500 |
| First session, mean of 3 rounds | 15.445 | 15.000 |
| Second session, rounds 1 to 8 | 12.833, 15.500, 16.000, 13.667, 16.500, 14.333, 16.333, 11.667 | 7.500, 14.667, 15.833, 15.333, 15.667, 16.000, 14.667, 13.167 |
| Second session, mean of 8 rounds | 14.604 | 14.104 |
| Second session, median of 8 rounds | 14.917 | 15.000 |
| Second session, mean of rounds 2 to 8 | 14.857 | 15.048 |

The raw records are the session logs `session.txt` and `session-terra2.txt` and the files `floor-r<round>-<revision>.txt`, `terra-r<round>-<revision>.log`, and `terra2-r<round>-<revision>.log`, which stay outside the repository.

### The decision

The change does not land. Its target is the Terra key press, and the two sessions show no gain beyond the session noise. The rounds of one binary span 11.667 ms to 16.500 ms. The mean of the second session falls by 0.500 ms, but round 1 of `4fc08860` (7.500 ms, 0.45 s of CPU time for 60 keys) is an outlier. Without it the after mean is 0.191 ms higher, and the medians differ by +0.083 ms. The floor shapes are all lower after the change, by 0.2 ns to 3.8 ns, and only the `map_get/2` shape, which moves a boxed map, falls by more than 1.5 percent. A fall of 2 ns to 4 ns for each boxed take gives 0.06 ms to 0.11 ms for the 27,472 takes of a key press, or less than 1 percent of 14.6 ms, which is below the resolution of the Terra harness. The attribution of task #625 gave 5.9 percent of the key press to the clone and the drop of `Move`. The measured fall is much smaller: in the samples of #625 the `Arc` count operations probably also hold the cache misses of the term loads, and a take keeps those loads. A static analysis of about 210 lines of loader code and comments, which each new instruction must keep correct, does not pay for a gain that the gate cannot show. Commit `c58b8789` reverts the change, and `docs/backlog.md` keeps it with a trigger.

### Known gaps

- The Terra harness cannot resolve a change below about 1 ms for each key, so the record does not bound the Terra effect of the change.
- The record does not measure the time of one take against one copy directly. The 2 ns to 4 ns estimate comes from the floor shapes.
- The count comes from a scratch probe build, not from the measured binary.
