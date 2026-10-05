---
name: draft-pr
description: This skill should be used when the user asks to "create a PR", "open a PR", "create a draft PR", "make a PR", "/pr", "/draft-pr", or otherwise requests a GitHub pull request for the current branch in the pbeam repo. Uses the task issue title as the PR title, fills the repo PR template with task links, decisions, and exact verification, checks each Review boundary item and stops to ask if one fails, applies content labels, updates the task issue and epic, and stops for review.
version: 0.4.0
user-invocable: true
allowed-tools:
  - Bash(git *)
  - Bash(gh *)
  - Bash(mise *)
  - Read
  - Write
---

# Draft PR

Open a GitHub PR from the current branch that follows `AGENTS.md`. One PR delivers one task issue. The PR body is the permanent record of decisions and verification.

Trigger phrases: "create a PR", "open a PR", "draft PR", "/pr", "/draft-pr". Open a ready PR by default, as the project history does. Open a draft only when the user asks for a draft.

## Writing for future readers

Reading only the PR, a reader must understand what changed, why, and how it was verified.

- **Self-contained.** Name the affected module, instruction, or flow.
- **Stable identifiers.** Issue numbers (`#49`), PR numbers, merge commits, code names (`Scheduler::deliver`), commands (`mise run check`).
- **Why over what.** Decisions carry rationale, alternatives, and consequences. The diff shows what.
- **ASD-STE100.** Short sentences. One instruction per sentence. Consistent terms. No filler.
- **No manual line-breaks.** Write each body paragraph or bullet as one unwrapped line; GitHub soft-wraps it.

## Hard rules

Refuse to continue, and say why, when a rule is broken.

1. **Never push to `main`.**
2. **Never force-push** unless the user explicitly asks.
3. **The working tree must be clean** for tracked files. If it is dirty, stop and ask. Do not commit, stash, or discard automatically.
4. **One task per PR.** The branch must implement one task issue. If the diff covers unrelated work, stop and propose a split.
5. **Prerequisites are merged.** Every blocker of the task issue must be closed and its PR merged. If not, stop.
6. **Title:** the exact task issue title, `[mN-eN] <Task title>`. See `references/conventions.md`.
7. **Body:** use `.github/pull_request_template.md`. Keep its sections and order. Fill every section with real content.
8. **Labels:** content labels only (`enhancement`, `documentation`, `bug`). No `kind:*` or `status:*` labels on PRs. Set the GitHub milestone of the task.
9. **No attribution lines** in the body, except a session link when the user's rules require one.
10. **Stop for review** after the PR is open. Do not merge. Do not start dependent tasks.

## Workflow

### 0. Read the task

Get the task issue number from the user, the branch, or the commits. Ask if it is unknown. Then read:

```bash
mise run issue <task> Acceptance Scope Dependencies
```

It prints the title, labels, milestone, the epic and milestone tracking issue links, and the named sections. From it, take the acceptance criteria, the scope, and the blockers. Read the task and PR lists of the epic with `mise run issue <epic> Tasks 'Pull requests'`.

### 1. Pre-flight

Run together:

```bash
git branch --show-current
git status --porcelain --untracked-files=no
git fetch origin main
```

Verify that the branch is not `main` and that tracked files are clean. Confirm the base (`main`) unless the user named another base.

### 2. Read commits and diff

```bash
git log origin/main..HEAD --no-merges --pretty=fuller --stat
git diff origin/main...HEAD --stat
git diff origin/main...HEAD --name-only
```

Commit bodies are the main source of rationale. Synthesise. Do not paste raw logs or diffs. Check that no scratch file (`docs/tmp/`, `portable_beam_runtime_handoff.md`, `.agents/.handoff/`) is in the diff.

### 3. Verify

Run `mise run check`, even for a documentation-only or skill-only change: it runs `rumdl check .`, which catches markdown lint failures that CI also runs. Record the exact commands and results, including `Application line coverage`. Run the acceptance check of the task (for example, the ERTS comparison test). If a check fails, stop and report. Do not open the PR with a failing check unless the user accepts it as a recorded gap.

### 4. Surface open questions

Look for:

- New `TODO`, `FIXME`, `XXX`, or `ponytail:` comments in the diff.
- Commit bodies with "open question", "TBD", "revisit", or "temporary".
- Acceptance criteria that the diff does not satisfy.
- Departures from ERTS behavior or from `docs/portable-beam-spec.md`.
- Ignored tests or lowered thresholds.

Show them and ask for each: `fix now`, `record in PR`, or `ignore`. `fix now` stops the flow. `record in PR` goes into `## Implementation decisions` or the known gaps in `## Verification`.

### 5. Draft the body

Follow the body rules in `references/conventions.md`. Write the body with the Write tool to a scratch file. Do not pass the body through a shell string.

### 6. Check the Review boundary

Evaluate each `## Review boundary` item from the template against the actual diff and task:

- This PR implements one task, with no unused foundations for later tasks.
- Acceptance criteria and known limitations are documented.
- Dependent implementation will wait for review and merge.

If an item is false (for example, the diff adds a field or function no current task needs, or an acceptance criterion is unmet with no documented gap), stop and ask the user: report which item fails and why, and let them choose `fix now`, `record in PR` (as a known limitation in `## Verification`), or `confirm override`. Do not check a box that is false and do not silently proceed past a failing item.

### 7. Push and create

```bash
git rev-parse --verify "origin/$(git branch --show-current)" >/dev/null 2>&1 \
  || git push -u origin HEAD
gh pr create \
  --base main \
  --title "<task title>" \
  --body-file <scratch body file> \
  --label <content label> \
  --milestone "<task milestone>"
```

Add `--draft` only when the user asked for a draft.

### 8. Update the issues

- Comment on the task issue: the PR link, the acceptance evidence, and that the task stays open until merge.
- Update the epic body: link the PR in the task bullet ("in review") and add the PR to the `## Pull requests` list.
- Do not close the task. Do not change milestone or epic completion.

Report the PR URL, labels, issue updates, and that the work stops for review.

## What not to do

- No PR title other than the task issue title.
- No sections outside the template.
- No placeholder text, HTML comments, or manual line-breaks in the body.
- No raw commit logs or diffs in the body.
- No claim of OTP compatibility beyond the verified fixtures.
- No merge, auto-merge, or dependent implementation.

## Additional resources

- `references/conventions.md`: title, label, and body rules (shared with `commit`).
- `AGENTS.md`: GitHub hierarchy, labels, and review rules.
