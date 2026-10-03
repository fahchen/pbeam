---
description: Capture recurring review problem classes from the human comments on one pbeam PR into docs/agents/knowledge.md.
---

# Capture review learnings from a PR

The caller gives two values before this prompt:

- `PR: #<N>`: the PR to analyse.
- `OUTPUT: <dir>`: a directory outside the repository for the result files.
  A local run without `OUTPUT` uses a new temporary directory.

Use the repository of the current checkout. This prompt only reads GitHub,
edits `docs/agents/knowledge.md`, and writes result files. It does not
commit, push, open PRs, or comment. The caller does that with the result
files (see `## 5. Publish the result`).

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

## 3. Write the entries

- **Local run** (an agent session during the PR implementation): show the
  proposals and wait for the user confirmation, as Step 7 of the skill says.
  Write only the confirmed entries.
- **CI run** (`MODE: ci` is given): nobody can confirm. Write every entry
  that passes all checks. The review of the knowledge PR replaces the
  confirmation.

Write only the entries into `docs/agents/knowledge.md` (Step 8 of the
skill). If the capacity check needs pruning, do not write.

## 4. Write the result files

Write these files into `OUTPUT`:

- `comment.md` (always): the comment for the PR.
  - Rules captured: "Captured review learnings from this PR.", then the
    heading of each captured rule. The caller adds the link to the commit or
    the knowledge PR.
  - No rule captured: "No new review learnings.", then one line for each
    candidate with the check or user decision that rejected it. If there were
    no candidates, say that the PR had no human review comments or no
    recurring problem class.
  - Pruning needed: the candidate rules and the 5 prune candidates, with a
    request to the owner to choose.
- `pr-body.md` (only when rules are captured): the body of the knowledge PR.
  For each rule: the full proposal (entry, Why, Good, Bad), the overlap
  analysis, and links to the source review threads.
- `commit.md` (only when rules are captured): the commit message. Subject
  `docs: capture review learnings from #<N>`, a blank line, and a body that
  names each rule and its source threads. No attribution trailers.

## 5. Publish the result

The caller publishes the result. Every run ends with exactly one comment on
the PR.

- **CI run:** the workflow commits on branch `learnings/pr-<N>`, pushes,
  opens the knowledge PR with `pr-body.md`, and comments on the merged PR
  with `comment.md` and the link to the knowledge PR.
- **Local run:** after this prompt, commit the entries on the PR branch with
  the `commit` skill (message from `commit.md`), push, and post `comment.md`
  with the commit SHA as a PR comment.

## Limits

- Change only `docs/agents/knowledge.md`.
- Keep the Markdown style of the knowledge file: line length 80 or less, one
  blank line around headings.
- Write all text in ASD-STE100 Simplified Technical English.
