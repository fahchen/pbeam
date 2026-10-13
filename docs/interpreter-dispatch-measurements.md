# Interpreter dispatch measurements

This document is the measurement record of the interpreter dispatch work. It is not a task tracker. GitHub Issues are the source of task status, and epic [#506](https://github.com/fahchen/pbeam/issues/506) holds the steps, their links, and their state. The plan is `docs/interpreter-dispatch-plan.md`.

The document collects the numbers that the merged pull requests of steps 0 to 6 report. The text names the revision and the source of each number. Every number is either measured or estimated, and the text says which one.

Baseline revision: `078c119` (2026-10-12). Current revision: `d36f686`, the head of `origin/main`, which is step 6 merged as [#527](https://github.com/fahchen/pbeam/pull/527). Step 6 changes no runtime code, so the numbers of steps 0 to 5 hold at the current revision.

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
- Each step column comes from the runs of the step that owns the column, with the aggregation rule that the table states. Two neighbouring columns therefore come from different runs or different groups of runs, and their difference holds both the change of the code and the run-to-run noise of about 10 percent at 200000 iterations.

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

Measured with the pinned rustc 1.99.0: `Instruction` is 136 bytes, `Operand` is 40 bytes, `Term` is 40 bytes, and `BifId` is 4 bytes. Each arm of `Instruction` has the 136-byte stride of the largest arm, `BinaryGetBinary`, whose fields take 132 bytes. One iteration of a floor loop retires 4 instructions and reads 544 bytes of slots inside an 816-byte range. One iteration of an added-instruction loop retires 5 instructions and reads 680 bytes inside a 1088-byte range.

Measured: the run loop performs no byte-code decode on a visit, because the loader decodes the whole `Code` chunk one time. The only encoding-dependent work of a visit is the bounds check of the instruction index.

Measured estimate of the memory-system difference of the two record strides, as a proxy: the proxy runs as its own step of the benchmark script, without the dispatch benchmark beside it, and each number is the minimum of five runs. At 300000 records of 136 bytes, which is 40800000 bytes and the estimated size of one Terra redraw instruction stream, the difference is 2.09 ns for each record, or 8.38 ns for each floor-loop iteration with 4 retired instructions. That number is 5.0 percent of the measured 167.4 ns of one `call_only` floor-loop iteration.

The decision of the step follows the step 8 gate of the plan: a compact form must give at least 10 percent on the floor loop and must measure at most 32 bytes for each instruction. The 5.0 percent estimate is below the gate, and the 816-byte range of the floor loop is below 1.3 percent of the smaller L1 data cache of the host, so the loop is taken to be cache-resident. The step therefore keeps the decoded form and skips step 8. The residency is an assumption, and the host has no hardware counter to confirm it.

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

Command: `bash scripts/terra_latency.sh release target/terra-latency-progress.txt`. The measured revision is `ea4dd13`, which was the head of `origin/main` at the time of that run. The raw output is `target/terra-latency-progress.txt`. The harness appends each block with its label, the revision, the time, and the host load.

The harness runs each mode one time. The numbers below are one run of each mode, and not the minimum of several runs, so they carry the run-to-run variance that a single run has.

| Mode | Revision | Load | First frame | Key `j` | Key `j` | Key `k` | Key `q` |
| --- | --- | --- | --- | --- | --- | --- | --- |
| PBeam release | `ea4dd13` | `{ 4.14 4.57 6.08 }` | 0.57 s | 0.018 s | 0.038 s | 0.023 s | 0.039 s |
| Pinned ERTS, `erl +S 1` | `ea4dd13` | `{ 3.67 4.45 6.00 }` | 0.18 s | 0.007 s | 0.003 s | 0.007 s | 0.004 s |

The four key latencies of the release build are 0.018 s to 0.039 s. The pinned ERTS numbers of the same harness run are 0.003 s to 0.007 s. The first frame is 0.57 s on the release build and 0.18 s on ERTS.

This is the only end-to-end effect in this document that a reader can compare with other work. The harness sends the four keys two seconds apart (`KEY_GAP = 2.0`, `scripts/terra_latency.py:33`), which is the spacing of the design run below. The design record of the copy-on-write epic, `docs/copy-on-write-terms.md:292`, reports 0.13 s to 0.21 s for the same four keys at a revision that it does not name. The baseline record of step 0 reports 0.087 s to 0.105 s at `078c119`. The current measurement is below both ranges.

The measurement covers one revision and one host. It does not isolate the dispatch work from the term work, the input handling, or the render, and the plan states that the redraw is 99.96 percent of one key press (`docs/interpreter-dispatch-plan.md:23`). It is one run of each mode, so it is weaker evidence than the minimum-of-nine floor-loop numbers above. The two modes ran at a load of `{ 4.14 4.57 6.08 }` and `{ 3.67 4.45 6.00 }`, which is not an idle host.

## The cost of one operation, by kind

This section answers one question: what does one operation cost, by kind? The four tables below hold the measured cost of one interpreter instruction, of one part of the call path, of one term operation, and of one end-to-end run. Each table names its revision and its source. Each number is measured, or the table marks it as computed.

### The interpreter instructions

The local `call_only` floor loop at 200000 iterations is the reference. The step 0 column is the before run of [#521](https://github.com/fahchen/pbeam/pull/521) at `211a645`, the merge of [#520](https://github.com/fahchen/pbeam/pull/520). Each number is the minimum of the nine sub-runs of a probe over three interleaved benchmark runs, and the load of the three runs is `{ 2.41 4.95 5.69 }`, `{ 3.57 5.07 5.71 }`, and `{ 3.53 4.96 5.66 }`. The current column is the run of [#527](https://github.com/fahchen/pbeam/pull/527) at `ea4dd13` at load `{ 3.67 4.38 5.93 }`, and each number is the minimum of the nine sub-runs of a probe.

| Item | Step 0 `211a645` | Current `ea4dd13` |
| --- | --- | --- |
| The floor loop, for each iteration (the plan shape at the step 0 revision, and the local `call_only` shape at the current revision) | 505.6 ns | 167.4 ns |
| The average for each instruction of the loop (computed: the floor-loop number divided by the 4 instructions that one iteration retires) | about 126 ns | about 41.9 ns |
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

The measured revision is `c2f0341`, the head of the pull request branch. The base runs are on the clean tree of that commit, and each variant run is on the same tree with the scratch edit of the variant, which the record of the run marks with the line `the working tree differs from HEAD`. The measurements ran in the quiet off-peak window of 2026-10-13 08:49 to 08:58 JST on the host of the environment table above. Every run started with a one-minute load at or below 11.69 on the 10-CPU host, and the record of each run holds its clock, its JST time, and its load.

### The reference points

The harness measures two numbers in every build:

- `L`, the local `call_only` floor loop of `dispatch_probe` at 200000 iterations, in nanosecond for each iteration. The number is the minimum of the nine sub-runs of the probe.
- `I`, the added cost of one `gc_bif '+'` on the same loop at 200000 iterations, in nanosecond for each iteration. The number is the `plus_loop/2` number of the same run less `L` of that run.

Measured at `c2f0341`, the three base runs and their minimum and spread:

| Number | Run 1 | Run 2 | Run 3 | Minimum | Spread |
| --- | --- | --- | --- | --- | --- |
| `L`, ns for each floor-loop iteration | 162.929 | 161.754 | 162.723 | 161.754 | 1.175 |
| `I`, ns for each added `gc_bif '+'` | 70.643 | 72.136 | 71.281 | 70.643 | 1.493 |

One iteration of the floor loop retires the 4 instructions `label`, `test is_eq_exact`, `gc_bif '-'`, and `call_only` (step 6 of this document). The table below gives the share of one iteration for each part, and the same number divided by those 4 instructions for one instruction of the loop.

The ablation of a part that both `floor_loop` and `plus_loop` hold cancels in `I`, because it lowers both numbers by the same work. The measured `I` of `V2` to `V6` is 67.774 ns to 72.373 ns against the base 70.643 ns, and every number is inside the 1.493 ns spread of the base `I` plus the noise of one run. The ablation method therefore cannot attribute `I`, and the column "one instruction" is a computed division of the measured iteration number and not a second measurement.

### The method

The main method is an ablation. A scratch build of `src/runtime/interpreter.rs` or of `src/term.rs` neutralises one sub-step, and the harness measures `L` and `I` in that build. The share of the sub-step is the difference between the variant and the base build of the same session. The scratch variants are not committed, and this section names each one.

A micro-measurement covers a part that an ablation cannot separate. The micro-measurement is `prints_the_cost_of_every_instruction_part` in `tests/runtime/interpreter/dispatch_attribution_costs.rs`, which reads the private helpers beside the interpreter. Each micro number is the minimum of nine timed runs of 100000 operations, in nanosecond for one operation. The thirteen numbers below come from the three base runs; each number is the minimum of the three runs, and the spread of the three runs is at most 0.421 ns.

| Variant | Committed patch | Scratch edit | Part |
| --- | --- | --- | --- |
| base | none | none | The reference points `L` and `I` |
| `V2` | `V2-cursor-compare.patch` | Remove the per-iteration compare `current_frame(process).module_id != cursor.module.id` (`src/runtime/interpreter.rs:603`), because the floor loop keeps one module. | The module of the active frame and the cursor |
| `V3` | `V3-operand-checks.patch` | Read the X slot with no bound check and no set check in `read_operand` (`src/runtime/interpreter.rs:3986-3989`), and write the X slot with no `resize` in `write_operand` (`src/runtime/interpreter.rs:4023-4027`). | The operand and register access |
| `V4` | `V4-label-index.patch` | Index the `function_starts` table by the label number with a `Vec` in `src/beam/module.rs`, so `function_at_label` (`src/runtime/interpreter.rs:3119`) reads a slot instead of hashing the label with FNV-1a. | The function lookup of a frame creation |
| `V5` | `V5-inline-bif.patch` | Run `erlang:'-'/2` on two immediate integers inline in the `Bif` arm (`src/runtime/interpreter.rs:800-866`), so the loop skips `read_bif_arguments`, `match_context_byte_size`, `bif_slice`, and the `BifResult` match. | The BIF call overhead |
| `V6` | `V6-term-eq-immediate.patch` | Return from `Term::eq` (`src/term.rs:353`) before the pair stack when the two terms are both integers, both `nil`, or both atoms, and keep the heap pair stack for a deeper term. | The comparison pair stack |

The dispatch and the branch have no variant. A build cannot remove the fetch of the record or the match of the opcode without a different loop, so the micro-measurement `dispatch_fetch_and_match` gives a lower bound.

### The measurement

The record of one build is one run of the harness, and every run records the clock, the JST hour, and the load. The measurement is three interleaved runs of the base build and of each variant in one quiet session. The reported number of a cell is the difference of the minimum of the three variant runs and the minimum of the three base runs, and the spread is the largest minus the smallest per-round difference `base run less variant run`.

Every run started with a one-minute load at or below 11.69. The base runs started at `{ 3.98 5.53 7.97 }`, `{ 10.81 7.88 8.63 }`, and `{ 9.59 9.29 9.10 }`. The variant runs started at `{ 8.41 7.26 8.41 }` to `{ 11.69 9.73 9.36 }`. The load rose above 12 between runs, because the release build of each variant is the only heavy work on the host, so a later run of a build waited for the one-minute load to fall to 10 or below before it measured.

### The parts

The number in each cell is measured, or the cell reads `not measured`. A part with no ablation stays `not measured` for the iteration and for the instruction, and the micro-measurement column gives the cost of one operation of that part.

| Part | Sub-step | Method | ns for one floor-loop iteration | share of the iteration | ns for one instruction of the loop (computed: the iteration number divided by 4) | Run-to-run spread of the iteration number | Micro-measurement, ns for one operation |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1. The dispatch and the branch | The fetch of the record, the match of the opcode, and the branch | Micro-measurement; no ablation | not measured | not measured | not measured | not measured | `dispatch_fetch_and_match` 1.506 |
| 2. The operand and register access | The operand read, the X write, the bound check, and the set check | Ablation `V3` | -2.499 | -1.5 percent | -0.625 | 2.000 | `read_operand_x` 2.014, `read_operand_literal` 1.654, `write_operand_x` 5.750 |
| 3. The lookups | The module of the active frame and the cursor | Ablation `V2` | 9.182 | 5.7 percent | 2.295 | 4.474 | none |
| 3. The lookups | The function lookup of a frame creation | Ablation `V4` | 1.762 | 1.1 percent | 0.441 | 1.700 | `function_at_label` 2.678, `frame_creation` 6.340 |
| 3. The lookups | The label lookup, the export lookup, and the BIF identity | Not present in the floor loop | not measured | not measured | not measured | not measured | `label_index` 2.418, `bif_slice_minus` 5.815 |
| 4. The state | The frame position | Inside `V2` and `V4`; not separated | not measured | not measured | not measured | not measured | none |
| 4. The state | The reduction accounting and the switch-out check | Not present in the floor loop | not measured | not measured | not measured | not measured | `light_bif_switch` 0.426 |
| 4. The state | The process state and `fcalls` | Not present in the floor loop | not measured | not measured | not measured | not measured | none |
| 5. The BIF call overhead | The borrowed argument path, the identity dispatch, and the result match of the loop's `gc_bif '-'` | Ablation `V5` | 34.606 | 21.4 percent | 8.651 | 1.444 | `read_bif_arguments` 20.255, `bif_slice_minus` 5.815 |
| 6. The term work | The comparison pair stack of `Term::eq` | Ablation `V6` | 13.257 | 8.2 percent | 3.314 | 6.893 | `term_eq_integer` 21.522 |
| 6. The term work | The `Arc` clone and drop | Not present in the floor loop | not measured | not measured | not measured | not measured | `term_clone_drop_shared_tuple` 9.610, `term_clone_drop_immediate` 3.472 |

The measured rows sum to 34.9 percent of one iteration: 21.4 percent for the BIF call overhead, 8.2 percent for the comparison pair stack, 5.7 percent for the module of the active frame and the cursor, 1.1 percent for the function lookup, and -1.5 percent for the operand and register access. The rest of one iteration is the part with no ablation, the frame position, the body of the `test` and of the `call_only` after the ablations, and the arithmetic of the `gc_bif '-'`. Those parts are not measured, so the sum of the table is not the cost of one iteration.

What a neutralised sub-step may hide, for each row:

- Part 1: The micro-measurement walks the instruction records of a module that holds the floor-loop shape with a four-arm match, and it measures 1.506 ns for one operation. The run loop matches about 100 arms and holds the branch history of the loop, so the micro number is below the real dispatch cost. No ablation removes the fetch, the match, or the branch, so the part is not measured.
- Part 2: The `V3` ablation reads the X slot with no bound check and no set check and writes the X slot with no `resize`. It measured -2.499 ns for each iteration, which is slower and inside its 2.000 ns spread, so the check cost is not separable this way. The error path of the two checks never runs in the floor loop, so the ablation measures the check and the tag test and not the error text. The removal also changes the code layout, and the negative sign holds in the three rounds (-1.530 ns to -3.530 ns). The floor loop writes only x0 of a register file that holds x0 and x1, so the `resize` is already a no-op and the growth path of a new register is not measured.
- Part 3, the module and the cursor: `V2` removes the compare and the read of the active frame from the process together, so the 9.182 ns holds both and not the compare alone. The loop must keep the compare for a call that enters another module, because the refresh of the cursor reads the active frame. The micro-measurement does not separate one store.
- Part 3, the function lookup: `V4` replaces the FNV-1a `LabelMap` of `function_starts` with a `Vec` indexed by the label number, so the frame creation reads a slot instead of hashing the label. The `Vec` read and the read of the function vector stay, so the 1.762 ns is the hash and the probe and not the whole lookup. The micro-measurement confirms the size: `function_at_label` falls from 2.678 ns in the base build to 1.355 ns in the `V4` build, and `frame_creation` falls from 6.340 ns to 3.552 ns.
- Part 3, the absent lookups: The floor loop takes no fail label and no jump, so `label_index` does not run in the loop, and the loader resolves the identity of the `bif` import one time, so the run of that instruction reads no table. The micro-measurements give 2.418 ns for `label_index` and 5.815 ns for `bif_slice_minus` for one operation of a path that resolves them.
- Part 4: The `Bif` arm and the `CallOnly` arm of the run loop do not call `light_bif_switch`; only the `call_ext` path and `send` do. Thus the floor loop has no reduction accounting and no switch-out check of a light BIF, and the `return` at the end of the loop charges one reduction for 200000 iterations. The micro-measurement `light_bif_switch` gives 0.426 ns for one operation of the path that has it. A store of the frame position is inside `V2` and `V4`, and no micro-measurement separates one store.
- Part 5: `V5` runs `erlang:'-'/2` on two immediate integers inline in the `Bif` arm. It removes the `BifArguments` build, which the micro-measurement gives as 20.255 ns for one call of `read_bif_arguments`; the `match_context_byte_size` check; the `bif_slice` dispatch to the table entry, which the micro-measurement gives as 5.815 ns for one call; and the `BifResult` match. The operand read and the register write stay, so the 34.606 ns is the call overhead and not the arithmetic. The inline path also removes the fail-label branch of the `Bif` arm. The three rounds agree: the per-round difference is 34.131 ns to 35.575 ns.
- Part 6, the pair stack: `V6` returns from `Term::eq` before the pair stack when the two terms are both integers, both `nil`, or both atoms. It removes the allocation for the `test` of the floor loop and for each other immediate comparison, and it keeps the heap pair stack for a deeper term. The micro-measurement confirms it: `term_eq_integer` falls from 21.522 ns in the base build to 2.026 ns in the `V6` build. The loop difference is 13.257 ns and is below the 21.522 ns of one `Term::eq`, so part of the allocation cost is hidden by the pipeline of the loop. The spread of 6.893 ns is the largest of the table, and the per-round difference is 6.558 ns to 13.451 ns.
- Part 6, the `Arc`: The floor loop holds immediate terms, so it performs no `Arc` clone. The micro-measurements give 9.610 ns for the clone and drop of a shared tuple and 3.472 ns for the clone and drop of an immediate.

The issue says that a part with a large share becomes its own task. The measured shares above 5 percent are the BIF call overhead of the `gc_bif '-'` at 21.4 percent, the comparison pair stack at 8.2 percent, and the module of the active frame and the cursor at 5.7 percent. Task [#541](https://github.com/fahchen/pbeam/issues/541) owns the pair stack. The BIF call overhead is the work that step 4 and step 5 of `docs/interpreter-dispatch-plan.md` left on the pure leaf path: the `BifArguments` build and the `bif_slice` dispatch. The module of the active frame and the cursor is the residue of step 2. This section records the numbers; the owner decides which of the two unowned shares becomes a task.

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

## 5. Known gaps

- `Instruction::CallExternal` of a pure BIF still builds one `Vec<Term>` (`src/runtime/interpreter.rs:3182`, `call_arguments`). The measured set of step 5 is the `bif` instruction, and no step owns this path.
- The two small classification scans stay on the external call path: `PBEAM_BIFS` with 5 entries (`src/runtime/interpreter/bif.rs:977`) and `HEAVY_BIFS` with 7 entries (`src/runtime/interpreter/bif.rs:1029`). The scan of the `call_ext_only` shape is 1.24 ns and the largest single scan is 3.22 ns against a review limit of 5 ns. A `call_ext` of a PBeam BIF runs both scans, and a scratch run measured about 5.4 ns for that path, which is above the limit. That path is not a measured benchmark shape. The trigger to store the classification at load time is a PBeam BIF in a hot path.
- The pair stack of `Term::eq` (`src/term.rs:318`) is one of the two allocations that remain on the call path of step 3. No step of the plan owns it.
- The L1 data cache residency of step 6 is an assumption. It follows from the measured 816-byte slot range and the measured L1 data cache sizes of the host, and it is not a measurement. The host has no hardware performance counter for cache misses, because macOS has no `perf`, so the cache evidence is a proxy of the host memory system.
- The repository has no Terra benchmark. The 5.0 percent estimate of step 6 is the memory-system difference of the two record strides at the estimated size of one redraw stream, and it is not a measurement of the redraw.
- The run-to-run noise is about 10 percent at 200000 iterations. A single column of a summary table therefore carries that noise, and a change inside it is not a measured effect.
- The end-to-end measurement of section 3 is one run of each mode, and not a minimum of several runs.

## 6. What comes next

Step 7 ([#515](https://github.com/fahchen/pbeam/issues/515)) merged as `c6cfb3a` ([#535](https://github.com/fahchen/pbeam/pull/535)) and removed the two measured allocations on the drop path: the placeholder handle of `Map::drain_into` and the unconditional work vector of `Term::drop`. Its acceptance is the drop cost of an immediate, a tuple, a shared map, and a deep list, before and after, with no drop regression and no map operation regression above 2 percent. Its blocked-by link to step 6 is satisfied, because step 6 is merged as `d36f686`.

Step 9 ([#517](https://github.com/fahchen/pbeam/issues/517)) adds combined instructions for the two measured pair shapes, `test` with `gc_bif` and `gc_bif` with `call_only`. Its gate is a floor loop at or below 150 ns for each iteration, and the target for the Terra key latency at that point is at most 10 ms, against the measured 0.018 s to 0.039 s of section 3.

Step 8 ([#516](https://github.com/fahchen/pbeam/issues/516)) is skipped. The gate of that step needs at least a 10 percent gain on the floor loop and at most 32 bytes for each instruction. The step 6 estimate is 5.0 percent under the memory-system proxy, which is below the gate, and the 816-byte slot range of the floor loop is below 1.3 percent of the smaller L1 data cache of the host. The step number does not change, and step 9 follows step 7 without it. Step 9 keeps a blocked-by link to step 8, and that link is stale while step 8 is skipped.
