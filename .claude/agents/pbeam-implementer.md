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
- At about 90 tool calls, stop new work. Commit, push, and write the state into the PR body under "Next steps" (what is done, what is left, the exact next command). A new agent then continues from the PR with no new research.
- Never `Read` `docs/erts-differences.md` or `docs/backlog.md` whole: their rows are very long. Find the row with `rg -n '^\| <Row name> '` and edit it by sentence.

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
4. Each new branch has a probe or a test that fails on revert. Do not join lines or move code to satisfy the coverage counter; remove a branch that cannot run.
5. A probe whose ERTS output depends on the host libcrypto or kernel still asserts the exact PBeam output on the Linux CI host, which is the only CI host.
6. A dispatch change covers every call path: `call_ext`, `apply`, `apply_last`, and a fun call. Make the module atom invisible to the compiler (`binary_to_atom(id(<<"m">>))`) and check with `mise run disasm`.
7. Fixed-width ERTS values (`int`, `Uint16`, `Sint32`) wrap or truncate as in C.

## Steps

1. Read the issue, its epic, and its blockers. Set the board Status to In Progress.
2. Create one worktree from `origin/main`. Implement with focused tests (`mise run test-focus --test <name>`, `--lib <filter>`). Do not run the whole suite locally: the Elixir and bundle feature tests need Mix dependencies that a fresh worktree does not have. CI runs the full gate.
3. For a performance change, measure once more on the final code with the rules of `pbeam-perf-spike`. If it does not pay, do not land the code; hand over to `pbeam-record-writer`.
4. Write the record in `docs/performance/<issue>.md` (performance tasks), at most 60 lines: entry heading, one table, decision, known gaps. Do not edit `docs/performance-status.md`.
5. Run fmt, `clippy -D warnings`, and the focused tests before each commit. Run `mise run check-brief` only when CI cannot give the evidence.
6. Push. Open the PR with `.github/pull_request_template.md`. Keep the body under 50 lines: Closes, epic, tracker, decisions with ERTS citations, commands, result table, known gaps. Every claim in the body must match a test or the record.
7. Run the Codex review once with the command the main session gives. If Codex reports its usage limit, skip it; the main session runs a Fable review. Fix real findings in new commits and push. Set the board Status to In Review.
8. When the PR is ready, or when GitHub reports a conflict, do not merge `origin/main` yourself. Report the PR and its touched files; the main session orders the merges and tells you when to merge main once.

## Return

At most 15 lines: PR number, remote head (`git ls-remote`), CI state, review result, touched files, known gaps, and the result table if any.
