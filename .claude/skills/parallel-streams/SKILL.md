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
3. Start the first ready task of each stream with `pbeam-implementer`, `isolation: worktree`, and the model and effort that the owner set (see the memory or the session).

## Review gate

- A PR merges when a review gives "No findings." and CI is green. The owner accepted Codex and Fable as reviewers (2026-10-22). When Codex reports its usage limit, run `pbeam-erts-skeptic` with `model: fable`.
- After fixes, run a delta review: name the previous head, so the skeptic reads only the new diff, plus the callers and tests of added code.
- Skip the re-review when a fix changes only tests, comments, or the PR body exactly as the reviewer asked. Fix a prose-only finding (a wrong count or citation in the PR body) yourself with `gh pr edit`.
- A finding that widens the scope becomes a new task under the right epic, with a blocked-by link. Comment on the PR.

## Merge queue

- Agents never merge `origin/main` on their own. When a PR is ready or conflicts, the agent reports its PR number and its touched files (`gh pr diff <n> --name-only`).
- Merge ready PRs whose files do not overlap first. A PR that overlaps a queued PR waits; after the earlier PR merges, tell its agent to merge main once.
- Before a merge, read `mergeable`. GitHub often reports `UNKNOWN` for a few seconds after a push; read it again. A squash merge with the PR head SHA (`gh api -X PUT repos/<repo>/pulls/<n>/merge -f merge_method=squash -f sha=<head>`) fails safely on a conflict.
- When main moved after a PR's CI and the PR changes code next to a merged PR, run `cargo check --all-targets` on a local merge before the squash merge.
- A conflict only in `docs/erts-differences.md` or `docs/backlog.md`: resolve it yourself. These rows are very long. Merge by sentence: take the base, ours and theirs of each conflicting line, and apply each side's changed sentences to the other side. Then run `cargo check` and `mise run lint`, push, and wait for CI.

## After each merge

1. Close the task if `Closes` did not close it. Set the board Status to Done. Find the board item with `gh api graphql` on the issue's `projectItems`, because `gh project item-list` misses items.
2. Tick the task in its epic, add the PR to the epic's `## Pull requests`, and close the epic and tick it in the tracker when all its tasks are done.
3. Remove `status:blocked` from the tasks that the merge unblocks.
4. Remove the agent worktree (`git worktree remove -f -f <path>`).
5. Start the next ready task of that stream.

## Budgets and restarts

- An implementer stops at about 90 tool calls and writes "Next steps" in the PR body. Start a new implementer in the same worktree with those steps; do not resume an agent that hit its budget.
- After an API session limit, resume each stopped agent with `SendMessage` once the limit resets.
- Keep reports short: ask for at most 15 lines. Do not read an agent's transcript file.

## Owner decisions

Stop and ask the owner only for a real product or design decision (for example a new dependency, an ADR for new `unsafe`, a platform behavior). Record the question in the progress document, keep the other streams running, and let the blocked stream take another ready task.
