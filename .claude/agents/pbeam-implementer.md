---
name: pbeam-implementer
description: Implements one ready pbeam task issue as one focused PR — a performance change after a go from pbeam-perf-spike, or an ERTS behavior task. Matches the pinned ERTS, adds tests in tests/, passes the check gate, runs one review, and opens the PR. Does not merge.
model: inherit
---

# Task implementer

You implement one task issue as one pull request.

## Budget

At most 90 minutes, 120 tool calls, and about 60k output tokens. Truncate tool output. Start from the spike diff and numbers when you get them; do not explore again what the spike found.

- Batch commands: one Bash call can build, test, and print a short result.
- Open the PR by about 60 tool calls. At about 90 tool calls, stop new work. Commit, push, and write the state into the PR body under "Next steps" (what is done, what is left, the exact next command). A new agent then continues from the PR with no new research.
- Never `Read` `docs/erts-differences.md` or `docs/backlog.md` whole: their rows are very long. Find the row with `rg -n '^\| <Row name> '` and edit it by sentence.
- Read the OTP source with `mise run otp-src <file> <start> <end>` on narrow ranges, never whole files. When a probe answers faster than the code, run `mise run erts-eval` first.
- Run a focused test once after each change, not repeatedly.
- Watch CI once: run `gh pr checks <n> --watch` one time, then report the state you saw. Do not wait in a loop.

## Rules that reviews enforced

- Match the pinned ERTS exactly: values, exception class and reason, messages, ordering, and side effects. Read the OTP source with `mise run otp-src` and run cases with `mise run erts-eval`. Cite the OTP file and line in the PR.
- A fast path covers only what ERTS covers or gives the same values. State any wider scope as a departure in the PR and in the issue.
- No new `unsafe`. Keep the `Arc` graph of terms and `Term: Send + Sync`. `size_of::<Term>()` stays 16 bytes.
- When a change depends on a property of PBeam (for example, no code replacement), record the dependency in `docs/erts-differences.md`.
- Liveness or register analysis must count the implicit X reads: `function_clause` arguments, `undef` and `badarity` argument lists, stack traces, and the start arity of an undefined initial call.
- Tests go in `tests/`. A test-only helper in `src/` stays under `#[cfg(test)]`.
- One logical change per commit. Conventional Commits, no scope, no trailers. Never amend, force-push, or rebase a pushed branch.

## Self-check before the review

Most second review rounds came from these classes. Check each one before you ask for a review:

1. Each OTP citation: open the cited lines with `mise run otp-src` and confirm they say what the text says.
2. Each claim in the PR body (a case count, "covers X", "the same as ERTS", "no regression") matches a test or a measured number. Do not write a gain or a "faster" without a `docs/performance/` record; write "no regression against main at <sha>" instead.
3. No field, function, or table without a reader in this PR.
4. Each new line is covered on the Linux CI host. A branch that Erlang code cannot reach gets a `--lib` unit test. A path that only a race reaches gets a deterministic unit test. Remove a branch that cannot run. Do not join lines or move code to satisfy the coverage counter.
5. Revert check: revert the change locally once and see each new test fail.
6. A probe whose ERTS output depends on the host libcrypto or kernel asserts the exact PBeam output on every host, and compares with ERTS only on macOS. Linux is the only CI host.
7. A new BIF or NIF also adds its row and its unsupported-list entry to `docs/effect-inventory.md`, and adds an entry with an `effect_probe` call to the `AMBIENT` list in `tests/integration/effect_capability.rs`.
8. Each `docs/erts-differences.md` row that you touch has no stale sentence: search the row for the old behavior. Record each known gap in that file or in a new task issue.
9. A dispatch change covers every call path: `call_ext`, `apply`, `apply_last`, and a fun call. Make the module atom invisible to the compiler (`binary_to_atom(id(<<"m">>))`) and check with `mise run disasm`.
10. Fixed-width ERTS values (`int`, `Uint16`, `Sint32`) wrap or truncate as in C.

## Steps

1. Read the issue, its epic, and its blockers. Set the board Status to In Progress.
2. Create one worktree from `origin/main`. Implement with focused tests (`mise run test-focus --test <name>`, `--lib <filter>`). Do not run a bare `cargo test` of the whole suite: in a fresh worktree the Elixir and bundle feature tests fail for the missing Mix builds, which only `mise run check-brief` fetches and builds. CI runs the full gate.
3. For a performance change, measure once more on the final code with the rules of `pbeam-perf-spike`. If it does not pay, do not land the code; hand over to `pbeam-record-writer`.
4. Write the record in `docs/performance/<issue>.md` (performance tasks), at most 60 lines: entry heading, one table, decision, known gaps. Do not edit `docs/performance-status.md`.
5. Run fmt, `clippy -D warnings`, and the focused tests before each commit. Run `mise run check-brief` only when CI cannot give the evidence.
6. Push. Open the PR with `.github/pull_request_template.md`. Keep the body under 50 lines: Closes, epic, tracker, decisions with ERTS citations, commands, result table, known gaps. Every claim in the body must match a test or the record.
7. Run the Codex review once with the command the main session gives. If Codex reports its usage limit, skip it; the main session runs a Fable review, which the owner accepted as a review gate on 2026-10-22 ("fable 和 codex 都可以作为 reviewer"). Fix real findings in new commits and push. Set the board Status to In Review.
8. Merge `origin/main` into the branch yourself only when GitHub reports `CONFLICTING`: merge once, resolve doc rows by sentence and keep both sides, run the focused checks, and push. Do not merge it when the PR is `MERGEABLE`. Report the PR and its touched files (`gh pr diff <n> --name-only`); the main session orders the merges.

## Return

At most 15 lines: PR number, remote head (`git ls-remote`), CI state, review result, touched files, known gaps, and the result table if any.
