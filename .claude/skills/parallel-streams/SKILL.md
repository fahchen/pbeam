---
name: parallel-streams
description: This skill should be used when the user asks to run several pbeam milestones (or several independent epics) in parallel, "one worktree per milestone", "开干" on a set of open milestones, or otherwise wants the main session to drive many implementer agents at once. It sets up one work stream per milestone, runs one task at a time per stream, reviews each PR, and keeps one merge queue so that branches merge main only once.
version: 0.1.0
user-invocable: true
---

# Parallel milestone streams

The main session drives several milestones at once. Each milestone is one stream. Each stream runs one task at a time in its own worktree. Only the main session merges.

## Set up

1. The owner names the milestones that run in parallel. Each needs its GitHub structure first: milestone, tracker, epics, tasks, sub-issue links, blocked-by links, and board items. Use one planner agent for each milestone, in parallel. Planners edit no repository file.
2. Copy `references/implementer-prompt.md` to a scratch file (for example `/tmp/impl-common.md`). Each implementer prompt starts with "Read <that file> first", then gives only the task-specific part: the issue, the prior PRs it builds on, the files that a queued PR also touches.
3. Copy the review rules (budgets, worktree rule, reply format) to a second scratch file (for example `/tmp/review-common.md`). Each review prompt starts with "Read <that file> first".
4. For a large or unclear task, first run a read-only spec agent. It reads the OTP source and runs `mise run erts-eval`, then writes a spec file. Pass that file to the implementer. Split an oversized task into sub-issues before coding.
5. Start the first ready task of each stream with `pbeam-implementer`, `isolation: worktree`, and the model and effort that the owner set (see the memory or the session).

## Review gate

- A PR merges when a review gives "No findings." and CI is green. The owner accepted Codex and Fable as reviewers (2026-10-22). When Codex reports its usage limit, run `pbeam-erts-skeptic` with `model: fable`.
- After fixes, run a delta review: name the previous head, so the skeptic reads only the new diff, plus the callers and tests of added code.
- Skip the re-review only when a fix changes comments or the PR body exactly as the reviewer asked; a changed test still gets a delta review, because a test that does not fail on revert is a finding. Fix a prose-only finding (a wrong count or citation in the PR body) yourself with `gh pr edit`.
- A finding that widens the scope becomes a new task under the right epic, with a blocked-by link. Comment on the PR.

## Merge queue

- An implementer merges `origin/main` once only when GitHub reports `CONFLICTING` for its PR. When a PR is ready or conflicts, the agent reports its PR number and its touched files (`gh pr diff <n> --name-only`).
- Merge ready PRs whose files do not overlap first. A PR that overlaps a queued PR waits; after the earlier PR merges, read its `mergeable` again. Only on `CONFLICTING` does it merge main, once: the PR CI already tests `refs/pull/<n>/merge`, so a clean PR squash-merges after green CI with no extra main merge (owner rule of 2026-10-22).
- Before a merge, read `mergeable`. GitHub often reports `UNKNOWN` for a few seconds after a push; read it again. A squash merge with the PR head SHA (`gh api -X PUT repos/<repo>/pulls/<n>/merge -f merge_method=squash -f sha=<head>`) fails safely on a conflict. GitHub refuses that call for a PR in a stack; use `gh api -X PUT repos/<repo>/pulls/<n>/merge-async -f merge_method=squash -f merge_action=direct_merge` there.
- Before each squash merge, run `cargo check --all-targets` on a local merge with `origin/main`. A clean text merge can still fail to build, for example on an array length.
- When a PR is only stale (behind main, no conflict), update it with `gh api -X PUT repos/<repo>/pulls/<n>/update-branch`. Use `pbeam-merge-prep` only on a real conflict.
- To merge main into a conflicting PR, use `pbeam-merge-prep` (sonnet for a docs-only conflict), or the PR's own agent if it is still running. Merge-prep resolves conflicts in docs only, by sentence for the long rows of `docs/erts-differences.md` and `docs/backlog.md`, and stops on any other conflict; then the main session or the PR's agent resolves it, and the PR gets a delta review.

## After each merge

1. Close the task if `Closes` did not close it. Set the board Status to Done. Find the board item with `gh api graphql` on the issue's `projectItems`, because `gh project item-list` misses items.
2. Tick the task in its epic, add the PR to the epic's `## Pull requests`, and close the epic and tick it in the tracker when all its tasks are done.
3. Remove `status:blocked` from the tasks that the merge unblocks.
4. Remove the agent worktree (`git worktree remove -f -f <path>`).
5. Start the next ready task of that stream.

## Budgets and restarts

- An implementer stops at about 90 tool calls and writes "Next steps" in the PR body. Start a new implementer without `isolation: worktree`, give it the existing worktree path and those steps, and tell it to use no other worktree. Do not resume an agent past about 90 tool calls: its context costs more than a restart.
- Use smaller models (sonnet or haiku) for bookkeeping: board status, epic ticks, and docs-only merge-prep.
- After an API session limit, resume each stopped agent with `SendMessage` once the limit resets.
- Keep reports short: ask for at most 15 lines. Do not read an agent's transcript file.

## Owner decisions

Stop and ask the owner only for a real product or design decision (for example a new dependency, an ADR for new `unsafe`, a platform behavior). Set `status:needs-input` on the task and write the question in its issue body, as AGENTS.md says; add it to the session's progress document if there is one. Keep the other streams running, and let the blocked stream take another ready task.
