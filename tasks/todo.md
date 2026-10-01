# Portable BEAM PR checklist

Task detail and acceptance criteria: [plan.md](plan.md). IDs below are planning IDs, not GitHub PR numbers. All implementation remains pending. Add actual PR links as work opens; check a task only after review, merge, and recorded verification. Do not auto-merge. Begin dependent implementation only after prerequisite PRs merge.

## M0 — Pin the compatibility target

- [ ] 01 — Define the first Terra acceptance scenario (blocked on exact project/revision/scenario).
- [ ] 02 — Capture the ERTS reference run and initial compatibility inventory (after 01).
- [ ] Checkpoint — Review target, inventory, toolchains, and first fixture.

## M1 — Execute a tiny BEAM slice

- [ ] 03 — Load and execute one constant-returning function (after 02).
- [ ] 04 — Execute a bounded local/external call chain (after 03).
- [ ] 05 — Propagate the first runtime exception (after 04).
- [ ] Checkpoint — Review compiled-fixture execution, call chain, and negative cases.

## M2 — Cooperative processes

- [ ] 06 — Yield between two runnable processes (after 05).
- [ ] 07 — Send a message and wake a waiting process (after 06).
- [ ] 08 — Remove exited processes safely (after 07).
- [ ] Checkpoint — Review fairness, wakeup, and clean exit.

## M3 — Multiple schedulers

- [ ] 09 — Deliver cross-thread messages with exclusive ownership (after 08).
- [ ] 10 — Steal runnable work (after 09).
- [ ] Checkpoint — Review exclusive ownership and observed work stealing.

## M4 — Linked native calls

- [ ] 11 — Dispatch one manually registered Rust function (after 10).
- [ ] 12 — Persist Elixir native declarations and verify supported invocation paths (after 11).
- [ ] 13 — Execute the native SHA-256 example (after 12).
- [ ] Checkpoint — Review declarations, digest, call paths, and failure behavior.

## M5 — Embedded application

- [ ] 14 — Round-trip a minimal versioned bundle (after 13).
- [ ] 15 — Run a hand-built embedded launcher (after 14).
- [ ] 16 — Collect the Mix application bundle (after 15).
- [ ] 17 — Generate and build the application launcher (after 16).
- [ ] Checkpoint — Review the moved executable in an ERTS-free environment.

## M6 — Safe asynchronous work

- [ ] 18 — Suspend for the first required host operation (after 17).
- [ ] 19 — Handle cancellation and late completion (after 18).
- [ ] Checkpoint — Review progress during wait, late completion, and shutdown evidence.

## M7 — Pinned Terra scenario

- [ ] 20 — Re-run Terra discovery and approve the next bounded gap task (after 19).
- [ ] 21 — Expand into one numbered sub-task/PR per discovered gap; no implementation until each sub-task has acceptance and verification (after 20).
- [ ] 22 — Prove the selected Terra release slice (after all required 21 sub-tasks).
- [ ] Checkpoint — Review Terra/ERTS parity, artifact independence, and precise compatibility claims.

## Per-PR handoff

Record the actual PR URL, task ID, baseline commit, commands/results, fixture input/output, and known limitations in each PR description. Update this checklist with its PR link. A failed acceptance check keeps the task open; a milestone checkbox is not a substitute for individual PR review.
