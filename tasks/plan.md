# Portable BEAM delivery plan

Status: proposed; no runtime implementation is authorized by this planning change.

Baseline: [runtime specification](../docs/portable-beam-spec.md). Execution checklist: [todo.md](todo.md). Task numbers below are stable planning IDs, not GitHub PR numbers.

## Review contract

- One numbered task becomes one focused PR. Every PR states the behavior added, explicit exclusions, dependencies, and reproducible verification evidence.
- Implement only the next ready task. Its prerequisite PRs must be reviewed and merged first. Stop for review after opening each PR; do not auto-merge or implement the entire milestone on one branch.
- A milestone is an acceptance checkpoint, not a single implementation PR. At its exit, review the working demo and remaining compatibility gaps before moving on.
- Each PR must leave the existing supported slice working. Add only the terms, instructions, metadata, configuration, and boundaries its fixture consumes. Logical boundaries in the spec do not require separate crates.
- Target small or medium PRs, normally 1–5 substantive files. If a task no longer fits one focused session, first split it into numbered sub-tasks in a planning PR. Do not hide an entire opcode family or OTP subsystem inside one task.
- Paths below describe likely ownership, not an instruction to scaffold directories. Reuse the structure established by the first consumer.
- Keep the task checklist as the execution record. Add the actual PR link when opened and mark complete only after merge and acceptance evidence; GitHub milestones may group those PRs without duplicating the detailed task text.

## Milestones and order

| Milestone | Tasks | Exit evidence |
| --- | --- | --- |
| [M0 — Pin the compatibility target](https://github.com/fahchen/pbeam/milestone/2) | 01–02 | Terra identity and first scenario, pinned toolchains, ERTS reference result, compatibility inventory |
| [M1 — Execute a tiny BEAM slice](https://github.com/fahchen/pbeam/milestone/3) | 03–05 | Compiled fixture returns a value, calls another function, and reports defined failures |
| [M2 — Cooperative processes](https://github.com/fahchen/pbeam/milestone/4) | 06–08 | Yielding, message wakeup, and exit with no lost runnable process |
| [M3 — Multiple schedulers](https://github.com/fahchen/pbeam/milestone/5) | 09–10 | Exclusive execution across scheduler threads and observed work stealing |
| [M4 — Linked native calls](https://github.com/fahchen/pbeam/milestone/6) | 11–13 | Declared native SHA-256 example, checked arguments, dispatch and registration failures |
| [M5 — Embedded application](https://github.com/fahchen/pbeam/milestone/7) | 14–17 | Mix builds a moved executable that runs without ERTS or loose application files |
| [M6 — Safe asynchronous work](https://github.com/fahchen/pbeam/milestone/8) | 18–19 | Other processes progress during a wait; late completion and shutdown are safe |
| [M7 — Pinned Terra scenario](https://github.com/fahchen/pbeam/milestone/9) | 20–22 | Discovery-sized compatibility PRs and observable parity with ERTS |

Default delivery order is the table order. Dependencies below identify technical prerequisites; changing the delivery order requires updating the plan in a reviewed PR. M0 is a real gate: Terra's repository, revision, and scenario are currently unknown. Do not invent them or claim fixture work proves Terra compatibility.

## M0 — Pin the compatibility target

### 01 — Define the first Terra acceptance scenario

**Dependencies:** none. **Scope:** small, documentation only.

Identify the intended Terra project with the owner. Record repository URL, immutable revision, dependency lock state, entry point, inputs, observable output/side effects, and shutdown condition. Select the first OS/architecture and exact OTP/Elixir/Rust toolchains needed to reproduce it. Record any unavailable information as a blocker, not an assumed value.

**Acceptance:** the scenario can be described without “run Terra”; expected behavior and exclusions are explicit; the owner confirms this is the intended project and slice.

**Verification:** review the pinned sources and execute the scenario on ERTS if available; otherwise leave the task blocked with the precise missing input. No runtime compatibility claim.

**Likely files:** `docs/terra-target.md`, runtime specification's unresolved-details section.

### 02 — Capture the reference run and initial compatibility inventory

**Dependencies:** 01. **Scope:** medium.

Add the smallest reproducible reference scenario and capture its ERTS result. Record required modules and observed instruction/BIF/native/host dependencies, distinguishing static candidates from operations exercised by the scenario. Keep secrets and third-party source out of fixtures unless redistribution is permitted.

**Acceptance:** pinned inputs reproduce the reference result; an inventory points to evidence and known uncertainty; select the tiny fixture and exact instruction/term subset for task 03.

**Verification:** rerun the documented reference command from a clean checkout; inspect fixture disassembly using the pinned OTP toolchain. Record commands, versions, and expected results.

**Likely files:** `fixtures/reference/`, `scripts/reference.*`, `docs/compatibility.md`.

**M0 checkpoint:** approve target, inventory, and initial fixture before implementing the runtime. Unknown Terra identity blocks this milestone.

## M1 — Execute a tiny BEAM slice

### 03 — Load and execute one constant-returning function

**Dependencies:** 02. **Scope:** medium.

Create the minimum Rust package and a compile-to-BEAM fixture returning an atom or small integer. Parse and validate only the required chunks, decode its actual instructions once, resolve its entry MFA, and execute it through the interpreter. Keep loader, decoded instructions, and execution responsibilities recognizable without creating unused crates.

**Acceptance:** the compiled fixture returns the expected term; truncated input, incompatible format, and an unsupported opcode produce useful errors; no hard-coded replacement for fixture execution.

**Verification:** a focused `cargo test` exercises the valid BEAM and malformed/unsupported inputs; `cargo build` succeeds. Record the exact fixture rebuild command and supported OTP version.

**Likely files:** `Cargo.toml`, `src/lib.rs`, `src/beam.rs`, `tests/constant.rs`, `fixtures/constant.erl` (plus Cargo's generated lockfile).

### 04 — Execute a bounded call chain

**Dependencies:** 03. **Scope:** medium.

Extend the fixture to call one local function and one startup-loaded external MFA. Add only the required register/stack and call/return behavior; shared code remains immutable. Introduce an `Arc`-backed compound term only if this fixture actually requires one.

**Acceptance:** arguments and return values survive both call paths; nested returns restore execution correctly; unresolved MFA names the missing function.

**Verification:** focused Rust tests execute the compiled call fixture and compare results with ERTS, including an unresolved external call.

**Likely files:** existing loader/runtime modules, `tests/calls.rs`, `fixtures/calls.erl`.

### 05 — Propagate the first runtime exception

**Dependencies:** 04. **Scope:** small or medium.

Choose one failure exercised by the pinned slice, such as bad argument or arithmetic failure. Define the supported exception/result behavior and entry-process exit status; implement only the instructions and BIF needed to reach that failure.

**Acceptance:** successful and failing execution have defined distinct results; the error identifies its execution context; unrelated Rust panics are not used as ordinary BEAM errors.

**Verification:** compare the fixture's supported error class/reason with ERTS and test that valid execution still succeeds. Do not claim catch/try support unless implemented and exercised.

**Likely files:** runtime module, `tests/errors.rs`, failure fixture, `docs/compatibility.md`.

**M1 checkpoint:** demo execution from a freshly compiled BEAM, call chain, and negative cases. Review actual supported instructions before adding more.

## M2 — Cooperative processes

### 06 — Yield between two runnable processes

**Dependencies:** 05. **Scope:** medium.

Introduce process-owned execution state and one scheduler thread. Run two processes through a reductions budget; preserve registers and continuation between turns. Use a bounded fixture or host-side test to observe progress without introducing a metrics system.

**Acceptance:** both processes progress; yielding preserves results; one long-running process cannot monopolize the scheduler.

**Verification:** deterministic scheduler tests with a small budget check progress and resumed results; existing interpreter tests pass.

**Likely files:** runtime/process/scheduler modules, `tests/yielding.rs`, fixture if needed.

### 07 — Send a message and wake a waiting process

**Dependencies:** 06. **Scope:** medium.

Add only the spawn/send/receive behavior required by a ping-pong fixture. A waiting receiver leaves the runnable queue and returns when a matching supported message arrives. If full selective receive is not needed, document the exact supported receive shape and reject unsupported bytecode.

**Acceptance:** ping-pong returns the ERTS reference result; send-before-wait and send-during-wait do not lose wakeups; immutable message values are safe to share.

**Verification:** compiled fixture comparison plus focused scheduler interleaving tests; no sleep-based assertion of correctness.

**Likely files:** process/scheduler/runtime modules, `tests/messages.rs`, `fixtures/messages.erl`.

### 08 — Remove exited processes safely

**Dependencies:** 07. **Scope:** small or medium.

Define the initial process-exit and runtime-shutdown policy for this fixture. Remove a terminated process from runnable/waiting ownership and handle a message targeting it predictably. Links and monitors remain deferred unless the selected scenario requires them.

**Acceptance:** exited processes never run again; sending to an exited process follows the documented supported semantics; the fixture shuts down without stranded scheduler work.

**Verification:** deterministic tests cover exit while runnable, exit while waiting, and message after exit; rerun ping-pong.

**Likely files:** process/scheduler modules, `tests/process_exit.rs`, compatibility notes.

**M2 checkpoint:** demonstrate fairness, mailbox wakeup, and clean exit on one scheduler.

## M3 — Multiple schedulers

### 09 — Deliver cross-thread messages with exclusive ownership

**Dependencies:** 08. **Scope:** medium.

Run a process on each of two scheduler threads with thread-safe inbox delivery. Keep process mutation owned by exactly one scheduler; share immutable code. Do not introduce migration yet.

**Acceptance:** cross-thread ping-pong completes; a process cannot execute simultaneously on two threads; shutdown joins both schedulers.

**Verification:** instrumented test asserts exclusive execution and message delivery under repeated interleavings; run the one-thread cases unchanged.

**Likely files:** scheduler/process modules, `tests/multischeduler.rs`.

### 10 — Steal runnable work

**Dependencies:** 09. **Scope:** medium.

Allow an idle scheduler to take runnable work from another scheduler. Transfer ownership only at a runnable boundary; waiting and exited processes cannot be stolen.

**Acceptance:** a deliberately imbalanced workload executes on both threads; no process runs twice concurrently or disappears; repeated yield/message/steal cycles finish correctly.

**Verification:** deterministic ownership assertions plus a bounded stress test, including exit and message arrival around transfers. Record the mechanism used to prove a steal occurred.

**Likely files:** scheduler module, `tests/work_stealing.rs`, existing multithreaded tests.

**M3 checkpoint:** review ownership transitions and demonstrate work stealing; throughput optimization is outside this milestone.

## M4 — Linked native calls

### 11 — Dispatch one manually registered Rust function

**Dependencies:** 10. **Scope:** medium.

Add a manual MFA registry and checked argument/result contract for one tiny synchronous function. External calls check native registration before BEAM resolution. Define duplicate registration and missing-function diagnostics before executing the fixture.

**Acceptance:** registered MFA reaches Rust; unregistered ordinary MFA still resolves to BEAM; invalid arguments and conflicting registration fail predictably.

**Verification:** compiled fixture exercises native and BEAM dispatch; focused negative tests verify the error contract without unchecked indexing or conversion.

**Likely files:** native/runtime modules, `tests/native.rs`, native fixture.

### 12 — Persist Elixir native declarations

**Dependencies:** 11. **Scope:** medium.

Implement `use PBeam.Native` and `native :name, arity` for the supported declaration form. Generate the ERTS stub and inspect persisted native MFA metadata from compiled BEAM. Audit local calls, dynamic application, and function references against the selected slice; define supported paths explicitly.

**Acceptance:** metadata survives normal compilation; the stub raises `:pbeam_native_only` on ERTS; every invocation path claimed supported reaches the registered implementation, with gaps recorded rather than silently running the stub.

**Verification:** focused Mix tests inspect metadata and stub behavior; compiled native-call fixtures run on PBeam for each claimed path. Split additional call-path implementation into reviewed sub-tasks if needed.

**Likely files:** `mix.exs`, `lib/pbeam/native.ex`, Elixir tests, native metadata reader/test.

### 13 — Execute the native SHA-256 example

**Dependencies:** 12. **Scope:** medium.

Add the binary argument/result representation required by the example and a statically linked hash registration. Use an established implementation instead of implementing cryptography. Validate the declared native set against registrations before application execution.

**Acceptance:** declared SHA-256 returns a known digest; non-binary input and missing registration fail before unsafe execution; binary ownership remains valid across the call.

**Verification:** known digest vector, malformed argument, missing implementation, and declaration-to-registration tests. Do not imply general bitstring matching support.

**Likely files:** binary/native modules, hash example crate, native tests, dependency manifest.

**M4 checkpoint:** demo the declared native example and negative cases; review the call-path support table.

## M5 — Embedded application

### 14 — Round-trip a minimal versioned bundle

**Dependencies:** 13. **Scope:** medium.

Define and implement the smallest bundle consumed by the existing fixtures: format/compiler identity, entry MFA, modules, one resource, and payload boundaries. Choose explicit size/count limits and duplicate-entry rules. Compression remains deferred unless measured need justifies it.

**Acceptance:** a bundle round-trips its contents; invalid versions, lengths, duplicates, and incompatible compiler metadata are rejected; resource lookup has documented semantics.

**Verification:** round-trip and malformed-input tests exercise the real reader/writer with bounded allocations.

**Likely files:** bundle module, `tests/bundle.rs`, `docs/bundle-format.md`.

### 15 — Run a hand-built embedded launcher

**Dependencies:** 14. **Scope:** medium.

Embed one complete bundle with `include_bytes!`, register the linked native example, finish validation/initialization, then spawn the entry MFA. Establish startup arguments and exit status for this executable only.

**Acceptance:** the embedded fixture calls Rust and reads the bundled resource; initialization errors happen before entry execution; moving the executable away from source/build outputs preserves behavior.

**Verification:** build and run the example in a clean directory with no loose BEAM/resource files; capture result and exit status. Inspect target dynamic dependencies separately from the ERTS-free claim.

**Likely files:** launcher example, embedded fixture bundle/build helper, executable smoke check.

### 16 — Collect the Mix application bundle

**Dependencies:** 15. **Scope:** medium.

Add the build-side collector for the known example: application and dependency BEAM modules, explicitly required OTP modules, resource files, and native declaration metadata. Reuse the bundle format. Record inclusion rules; do not silently assume a static call scan finds dynamic dependencies.

**Acceptance:** the example builds a bundle from its normal Mix compilation; all fixture-required modules/resources/native declarations are present; missing inputs and duplicate modules have actionable diagnostics.

**Verification:** focused Mix tests inspect the generated bundle and feed it to the existing reader/launcher path. Document commands and inputs.

**Likely files:** Mix collector/task modules, focused Mix tests, example configuration.

### 17 — Generate and build the application launcher

**Dependencies:** 16. **Scope:** medium.

Complete `mix pbeam.build` by generating the launcher Cargo dependencies/registration calls from the example's configured native paths, embedding the collected bundle, and building the selected release target. Restrict the first implementation to the target chosen in M0.

**Acceptance:** one command produces the executable; configured native code is linked with no extension library distribution; the moved executable runs without installed Erlang/Elixir or accompanying application files.

**Verification:** clean build plus a clean target environment with no ERTS, compare output/digest/resource content and exit status. Record system-library dependencies and avoid claiming other platforms are supported.

**Likely files:** Mix build task/launcher generation, integration test, build documentation.

**M5 checkpoint:** review an actual delivered artifact and reproducible clean-environment evidence.

## M6 — Safe asynchronous work

### 18 — Suspend for the first required host operation

**Dependencies:** 17. **Scope:** medium.

Choose one operation from the M0 inventory. Define continuation ownership, completion identity, and a minimal host execution boundary before implementing it. Blocking work runs off scheduler threads; completion returns through the scheduler inbox. Choose an executor only when this operation needs it.

**Acceptance:** another process progresses while the caller waits; the call resumes exactly once with its result/error; the test covers completion before and after suspension is registered.

**Verification:** controlled host-completion tests exercise both orderings and error propagation; the fixture proves scheduler progress without relying on timing alone.

**Likely files:** host/native/scheduler integration, `tests/async_host.rs`, one operation fixture.

### 19 — Handle cancellation and late completion

**Dependencies:** 18. **Scope:** small or medium.

Complete the lifecycle contract for that same operation: caller exit, cancellation where supported, and runtime shutdown. Specify whether host work is cancelled or its result discarded; resources must be released either way.

**Acceptance:** late or duplicate completion cannot revive a dead process or resume twice; completion cannot target a reused process/call identity; shutdown has a defined bounded behavior for outstanding work.

**Verification:** deterministic tests force exit/completion, cancellation/completion, and shutdown/completion orderings and assert ownership/resource release.

**Likely files:** existing host/scheduler modules, async lifecycle tests, lifecycle contract notes.

**M6 checkpoint:** review the lifecycle state transitions and failure evidence before adding more asynchronous services.

## M7 — Pinned Terra scenario (discovery gated)

### 20 — Re-run Terra discovery against the delivered runtime

**Dependencies:** 19. **Scope:** small or medium, primarily evidence/planning.

Run the pinned scenario, compare against the M0 inventory, and identify the earliest unsupported observable behavior. Rank gaps by their dependency order. Turn the next gap into a concrete task 21 sub-task with a minimal reproducer, expected result, exact semantics, files, and verification before implementation.

**Acceptance:** every observed failure is distinguished from unexecuted coverage; the next PR has bounded scope; no blanket “implement remaining opcodes/OTP” task is approved.

**Verification:** attach reproducible failure evidence and the corresponding ERTS result; review the updated compatibility table and sub-task before coding.

**Likely files:** `docs/compatibility.md`, one reduced fixture, this plan/checklist.

### 21 — Close one discovered compatibility gap per PR

**Dependencies:** 20, then the previous accepted gap PR. **Scope:** small or medium per sub-task; not a single implementation PR.

This is a gated expansion slot, not authorization to implement an unknown feature set. Add IDs `21.1`, `21.2`, etc. only after discovery. Each names one observable capability and its minimal instruction/BIF/term additions. Candidate categories include selective receive, catch/try, links/monitors, bitstring operations, application startup, or a host service; none is mandatory until evidence demands it.

**Acceptance for each sub-task:** the reproducer matches ERTS; its negative/boundary case has a defined result; Terra advances past the identified gap without regressing previously accepted behavior.

**Verification for each sub-task:** focused test plus the pinned scenario to its next failure; publish supported-version and remaining-gap updates. Non-byte-aligned binary slices require explicit offset/length boundary checks when first needed.

**Likely files:** only the responsible existing subsystem, reduced fixture/test, compatibility table. More than five substantive files or multiple independent behaviors means split again before implementation.

### 22 — Prove the selected Terra release slice

**Dependencies:** all task 21 sub-tasks required by the pinned scenario. **Scope:** small or medium, validation and delivery documentation.

Build Terra using the accepted pipeline and compare its selected scenario with ERTS in the clean target environment. Publish exact version/platform scope, reproducible artifact instructions, and remaining exclusions.

**Acceptance:** observable result and shutdown match the agreed reference; executable runs without ERTS/loose application files; compatibility claims cover only demonstrated behavior.

**Verification:** run the documented release check from a clean checkout and execute the moved artifact; review results against all applicable specification acceptance checks.

**Likely files:** scenario integration check, `docs/compatibility.md`, release/build instructions.

**M7 checkpoint:** owner reviews the Terra evidence before calling the first application slice supported. Additional platforms, OTP coverage, performance work, and new scenarios require new scoped plans.

## Risks and deferred decisions

| Risk/open decision | Resolution point |
| --- | --- |
| Terra identity and version are unknown | Task 01 blocks implementation; request exact project/scenario rather than guessing |
| Compiler output contains more instructions than the tiny fixture suggests | Task 02 disassembly; shrink the fixture or split task 03 before coding |
| Native local/dynamic/function-reference calls bypass external dispatch | Task 12 call-path audit; explicit support tests and follow-up slices |
| Sharing terms across schedulers hides mutable ownership | Tasks 07–10 ownership assertions; only immutable payload sharing |
| Static module collection misses dynamic calls or startup needs | Task 16 inclusion contract; task 20 scenario evidence and explicit module requirements |
| Async executor/lifecycle complexity expands the PR | Tasks 18–19; one operation first, split further if needed |
| Initial platform, bundle encoding, executor, and startup semantics are unsettled | Decide at their first consuming task; no unused portability or configuration framework |
