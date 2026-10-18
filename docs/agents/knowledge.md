# Review Conventions

Read this file before implementation. Record only problem classes that recur in review. Keep individual comments and one-time decisions in their task or PR.

## Domain concepts and code names drift apart

Use the same domain concepts across specifications, grammar, modules, and
identifiers. Choose names that describe the data and responsibility they own.

## Code boundaries follow operations instead of responsibilities

Group code by stable responsibility. Keep concepts with different owners in
separate modules, even when one operation uses them together.

## Format contracts remain implicit

Keep the supported binary grammar and layout near the code that reads it. Name
non-obvious format values in implementation code. Show supported forms with
their limits and representative byte examples.

## Function behavior lacks examples

Document every function with Rustdoc comments and a `# Examples` section. Show
representative input and output in Rust code fences. For parser functions, use
concrete byte forms. State important observable errors and limits.

## Runtime models depart from ERTS

Before you model a BIF, instruction, or term operation, run it on the pinned
ERTS and record the observable result: return value, exit reason shape, messages
sent, and limits. Follow that result strictly: do not copy a similar BIF, and
do not add a limit or shortcut that ERTS does not have; change the PBeam design
instead. Record only scope that is not implemented yet as an ERTS difference,
never a workaround.

## Tests pass without the behavior they claim

A test must fail when its claimed behavior breaks. Drive behavior through
compiled BEAM fixtures, and make each test double perform the step under test.
Assert exact values from a pinned ERTS run, not only types.

## Untrusted automation has excessive access

Run automation that processes untrusted text with least-privilege credentials
and an isolated writable workspace. Do not expose persistent credentials or let
untrusted input write outside its output directory.

## External identifiers lack trusted-origin validation

Validate the trusted origin of an external identifier before you take a local
number or name from it. Do not use data from an unvalidated identifier for a
local action or an error report.

## Compiled fixtures omit representative input shapes

Cover each supported input shape, including bare values and boundary sizes, in
compiled fixtures. Assert exact results against pinned ERTS. When the fixture
compiler cannot emit an input shape, such as a bitstring that is not a whole
number of bytes, cover it with a unit test instead.

## Equality uses semantic identity, not dispatch metadata

Define equality from the fields that identify the runtime value. Exclude fields
that only select code or optimize execution.

## Task scope expands beyond measured behavior

Implement only behavior that the task owns and that the measured path requires.
Leave other instructions and BIFs on the unsupported path until their task owns
them. Update the task scope and follow-up ownership before adding shared
decoding or fixture coverage.

## Changes exceed the measured task path

Implement only behavior that the current task measures and needs. Move
unmeasured or unrelated support to a separate task.

## Binary decoders drop required operands

Consume and validate every encoded operand even when runtime semantics do not use its value. Keep the skip behavior in one helper and preserve cursor and tag checks.

## Opaque resource fields lack ownership documentation

Document every struct field that wraps an opaque or owned resource. State the owner, the live representation, and the operation that releases it.

## Documentation must match the current branch

Describe only behavior that exists in the current branch. Mark future behavior as planned work, not as current behavior.

## Encoded examples match complete operands

Validate each encoded example against the actual encoder or disassembler. Include every operand and use the correct tag for each value.

## Exit cleanup is completed before errors return

Run every required exit cleanup and notification stage before returning an error. Preserve the first error and return it after cleanup completes.

## Invariant violations remain visible

Treat a broken runtime invariant as an explicit failure. Do not skip missing entries when skipping would hide a defect or leave runtime state inconsistent.

## Test process failures preserve diagnostics

Include the process status, standard output, and standard error in test failure messages. Display non-UTF-8 output without dropping the bytes needed to diagnose the failure.

## Documented commands omit required setup

Make each documented command runnable with all required setup inputs. Reuse the same setup shown by the validated command or fixture.

## Instruction stream construction mixes ownership

Choose one instruction-stream construction model for each opcode path. Either return the instruction for the outer loop to append, or append all required instructions in the path and continue. Do not mix both models.

## Public values can violate documented canonical forms

When a public type cannot enforce a canonical representation, document it as a runtime-produced form, not as a type invariant. State that callers must preserve the form when they pass values into runtime operations.

## Resumable work repeats or skips its budget

Check the reduction budget before each unit of work that can grow with the input and on every switch-out and completion path. Keep the position of the work across turns and traps. Do not rebuild completed work after a trap, and do not do uncharged O(N) work before a yield.

## Published claims lack evidence checks

Verify every acceptance, measurement, and comparison claim against its cited source before publishing it. State limits that weaken the claim. Name the baseline and record decisions that differ from the cited requirement.

## Concurrent cache initialization races

Serialize cache creation before moving a downloaded or generated directory into its shared cache path. Check the destination again inside the serialization boundary before the move.

## Persistent build outputs contaminate discovery

Discover generated inputs from the current build set, not from a persistent output directory. Remove stale artifacts or isolate each build before scanning.

## Complexity claims omit hidden work

Expand every documented complexity claim into its complete operation sequence. Include copying, sorting, allocation, traversal, and deterministic-order steps that can grow with input size. State the cost of each operand or phase when constant-time access also performs size-dependent work.

## Arguments are checked after capability authorization

Validate arguments before capability checks. Preserve invalid-input errors.

## Benchmark measurements lose the measured work

Keep benchmark setup outside timed runs and isolate counters to the measuring thread. Consume inputs and outputs with `black_box`, and verify compiled probes retain the work before publishing results.

## Structural tests become stale after enum growth

Make structural tests fail to compile when the represented enum gains a variant. Match every variant and name every field instead of using `..` or a fallback.

## Incremental cleanup follows authoritative storage

When cleanup removes items in bounded steps, account and drain the authoritative
storage that owns their terms. Do not walk stale indexes or links after a step
can remove their entries.
