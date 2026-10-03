---
description: Capture recurring review problem classes from the human comments on one pbeam PR into docs/agents/knowledge.md.
---

# Capture review learnings from a PR

The caller gives the PR number before this prompt, as `PR: #<N>`. Use the
repository of the current checkout.

Run the capture-learnings skill for that PR. Read
`.agents/skills/capture-learnings/SKILL.md`, its references, `AGENTS.md`, and
`docs/agents/knowledge.md` first. Follow the skill, with the changes below.

## 1. Collect human comments

Collect the comments that humans left on the PR: review threads, review
bodies, and PR comments. Use `gh api` and the GraphQL `reviewThreads` query.

- A comment is human only when its author type is `User`.
- Skip authors of type `Bot`, logins that end in `[bot]`, and Copilot
  reviewers.
- Replies in a thread, the PR description, and commit messages are context
  only. They show how each comment was resolved.

## 2. Check candidates

Find candidate rules only in the human comments: a comment that states a
problem class, or a question or correction that changed the code. Apply
Steps 2 to 6 of the skill to each candidate: abstraction, recurrence, Good
and Bad examples, overlap, and capacity.

## 3. Act on the result

Pick the mode from the PR state (`gh pr view <N> --json state,headRefName`):

- **Open PR** (local run during the implementation): show the proposals and
  wait for the user confirmation, as Step 7 of the skill says. Write the
  confirmed entries on the PR branch. Commit them with the `commit` skill
  as a separate `docs:` commit, and push. Then comment on the PR (see
  `## 4. Comment on the PR`).
- **Merged PR** (CI run): nobody can confirm. The review of a knowledge PR
  replaces the confirmation.
  - No candidate passes: do not change files.
  - At least one candidate passes, and no pruning is necessary: create
    branch `learnings/pr-<N>` from `main`. Write only the entries into
    `docs/agents/knowledge.md` (Step 8 of the skill). Commit with the subject
    `docs: capture review learnings from #<N>` and a body that names each
    rule and its source threads. Push the branch. Open a PR to `main` with
    the same title and the label `documentation`. For each rule, the PR body
    shows the full proposal (entry, Why, Good, Bad), the overlap analysis,
    and links to the source review threads. Comment on the merged PR only
    after the new PR exists.
  - Pruning is necessary: do not write.

## 4. Comment on the PR

Every run ends with exactly one comment on the PR, in both modes, whatever
the result:

- Rules captured: "Captured review learnings in <commit SHA or #new PR>.",
  then the heading of each captured rule.
- No rule captured: "No new review learnings.", then one line for each
  candidate with the check or user decision that rejected it. If there were
  no candidates, say that the PR had no human review comments or no
  recurring problem class.
- Pruning needed: the candidate rules and the 5 prune candidates, with a
  request to the owner to choose.

## Limits

- Change only `docs/agents/knowledge.md`.
- Never push to `main`.
- Do not add attribution trailers to commits.
- Keep the Markdown style of the knowledge file: line length 80 or less, one
  blank line around headings.
- Write all text in ASD-STE100 Simplified Technical English.
