---
name: next-task
description: This skill should be used when the user asks "what's next", "what's the next task", "what can run in parallel", "/next-task", or otherwise wants the ready task issues in the pbeam repo. Finds every open `kind:task` issue whose blockers are closed/merged, reports all ready tasks grouped by parallel-safe vs sequential, and after the user selects one, confirms before moving its project board Status to `Designing` and proposing a solution.
version: 0.5.0
user-invocable: true
allowed-tools:
  - Bash(gh *)
  - Bash(mise run issue *)
---

# Next Task

Find every ready task issue, not just one. Then, once the user picks one, confirm before touching board state, and move to proposing a solution.

## Step 1: find ready tasks (read-only)

1. List open `kind:task` issues: `gh issue list --repo fahchen/pbeam --label "kind:task" --state open --json number,title,labels,milestone -L 50`
2. For each candidate, read its body for "Blocked by" / "Dependencies" text: `mise run issue <N> Dependencies Delivery`. It prints the title, state, labels, the text before the first section, and the named sections. Older issues name blockers in a `Delivery` section or a `**Dependencies:**` line before the first section, so a `no section` line for one name is normal.
3. Collect every blocker number named, dedupe, check each blocker's state: `gh issue view <blocker> --repo fahchen/pbeam --json number,state`
4. A task is ready when every blocker it names is `CLOSED` and any prerequisite PR it names is merged. Verify directly; don't trust stale "ready" wording in the body.
5. Among ready tasks, flag which can run in parallel:
   - Same epic, sequential acceptance criteria, or one's output feeds another (check epic body ordering) → sequential, not parallel.
   - Different epics, or same epic but explicitly independent scope (no shared files/modules per task body "Scope" section) → parallel-safe.
6. Report every ready task: issue number, title, epic, milestone, one-line why-ready (which blocker merged, PR #). Group into "parallel-safe" vs "sequential". If nothing is ready, say so and name what blocks the nearest candidates.

## Step 2: user selects a task

Wait for the user to name which ready task(s) to work on. Do not pick for them.

## Step 3: confirm before moving board Status

Before any board write, ask the user to confirm the task selection and that board Status should move to `Designing` now. Only on explicit confirmation, run:

```bash
gh project item-list 2 --owner fahchen --format json --jq \
  '.items[] | select(.content.number == <ISSUE_NUMBER>) | .id'
gh project item-edit --id <ITEM_ID> --project-id PVT_kwHOAA0Ofs4BlkkX \
  --field-id PVTSSF_lAHOAA0Ofs4BlkkXzhkQ6mo --single-select-option-id a2d64336
```

(`a2d64336` = `Designing`. Other option ids if later needed: `Todo` 1d5a540f, `Pending Review` 8d836862, `In Progress` 5286f3c9, `In Review` 81390039, `Done` 8641c74e.)

## Step 4: propose a solution

Read the task issue, its epic, and milestone tracker. Draft a proposed approach: scope, key design decisions, alternatives considered, affected files, and verification plan. Include a key code snippet when one clarifies the approach (an existing function signature to extend, a new match arm, a fixture shape). Keep the proposal concise and readable; write it in ASD-STE100. Present it to the user and stop — do not start implementation yet.

When the user confirms the proposal:

1. Update the task issue body with the proposed solution: `gh issue edit <N> --repo fahchen/pbeam --body-file <scratch file>`. Replace any existing `## Proposed solution` section with the current one; the body holds only the final solution, not its history. Include the scope, the key decisions, and the key code snippet. Keep it concise; do not paste the full diff or restate the acceptance criteria.
2. If this changes or narrows what the issue originally asked for, leave a comment on the issue explaining the change and why: `gh issue comment <N> --repo fahchen/pbeam --body "<reason>"`. The comment is the change record; skip it only when the proposal merely fills in detail the issue already implied.
3. Move board Status to `Pending Review` (same `gh project item-edit` pattern, option id `8d836862`).

Status moves to `In Progress` only after the user confirms the proposal and coding begins.

## Scope

Steps 1 and 4's drafting are read-only until the user confirms the proposal. The issue body update and comment in step 4 happen only after that confirmation. Any board Status write (step 3 and the `Pending Review` move in step 4) requires explicit user confirmation first — never move Status silently. Do not start implementation, open PRs, or change issue labels; that is later work, out of this skill's scope.
