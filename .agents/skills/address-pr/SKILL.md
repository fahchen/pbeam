---
name: address-pr
description: This skill should be used when the user asks to "address PR comments", "address PR review comments", "respond to PR feedback", "go through PR review comments", "what comments are on this PR", "/address-pr", or otherwise wants to walk through and respond to GitHub pull request review threads on a pbeam PR. Full lifecycle - fetch unanswered threads, discuss one by one, reconcile decisions, implement, commit via the commit skill, push, post all replies in one batch, and update the PR and issues. GitHub is the only record; the skill writes no tracking file and never resolves threads.
version: 0.3.0
user-invocable: true
argument-hint: "[<PR#>] [only my comments | @<login>]"
allowed-tools:
  - Bash(gh *)
  - Bash(git *)
  - Bash(mise *)
  - Bash(cargo *)
  - Bash(erlc *)
  - Bash(fd:*)
  - Bash(rg:*)
  - Read
  - Edit
  - Write
---

# Address PR

Full lifecycle for GitHub review comments on a pbeam PR. Discuss and decide first, implement and push next, reply last. Reviewers see replies that point at pushed commits, not promises.

```text
Phase 1  fetch unanswered threads
Phase 2  discuss one by one, record decisions in the decision table
Phase 3  reconcile (conflicts, overlaps, open discussions)
Phase 4  implement (only `fix` decisions)
Phase 5  commit (hand off to the commit skill)
Phase 6  reply preview - one user gate for the whole batch
Phase 7  push
Phase 8  reply, then update the PR description and issues
```

**GitHub is the only record.** `AGENTS.md` forbids file-based task trackers. Do not write a tracking file. Keep the decision table in the conversation. After the session, the replies, commits, and PR description hold every decision. To resume, fetch the threads again: a thread is answered when its last comment is a reply from the PR author.

**This skill never resolves threads.** The reviewer verifies the fix or the reason and resolves the thread.

## Staying on the rails

After every user message, before you do anything else:

1. **Name the phase and thread** (for example, "Phase 2, thread 3 of 7").
2. **Classify the reply:**
   - **Decision** (`fix`, `keep`, a plan, or an edit): record it in the decision table and advance.
   - **Clarifying question:** answer it briefly, ask the original question again, and stay on the same thread.
   - **Tangent:** acknowledge it in one line, add it to the parking lot, and return to the current thread.
   - **Scope expansion** (for example, "also resolve the threads", "also refactor the module"): give the reason against it briefly, park real follow-ups, and return.
   - **Process change** (for example, "skip Phase 6"): accept it once only if it is safe. Flag it if it breaks a hard rule.
3. **Check the gate:**
   - Phase 1 to 2: 1 or more unanswered threads. With zero threads, stop.
   - Phase 2 to 3: every thread is `fix`, `keep`, or `deferred`.
   - Phase 3 to 4: no `discuss`, no contradictions, no unresolved overlaps.
   - Phase 4 to 5: every `fix` has a real `file:line`, or all decisions are `keep` (skip 4 and 5).
   - Phase 5 to 6: each `fix` has a commit SHA.
   - Phase 6 to 7: the user approved the batch (`yes`, `y`, `push`, `lgtm`).
   - Phase 7 to 8: `git push` succeeded, or there was nothing to push.
   - Phase 8 to done: every reply POST succeeded.
4. **If the gate is not met, do not advance.** Ask the gate question again.

### Decision table

Show the table at the end of Phase 2, Phase 3, and Phase 6:

```text
| # | Location                    | Author    | Status | Decision                         | Commit  |
| 1 | src/runtime/process.rs #13  | @fahchen  | fix    | rename pc to next_instruction    | 70476fc |
| 2 | src/runtime/process.rs #28  | @fahchen  | keep   | ERTS has no message count limit  | -       |
```

Status values:

- `open`: not discussed yet.
- `fix`: agreed change. The decision holds the plan.
- `keep`: no change. The decision holds the reason, which becomes the reply.
- `discuss`: needs user input. Blocks Phase 3.
- `deferred`: skipped this round. Does not block. Listed in the wrap-up.

### Parking lot

Keep out-of-scope items in a parking lot list in the conversation. In the wrap-up, propose a GitHub issue for each item that needs work. Create an issue only when the user agrees, and follow the hierarchy rules in `AGENTS.md`.

## Invocation

| Form | Behavior |
| --- | --- |
| `/address-pr` | Use the PR of the current branch. Walk all unanswered threads. |
| `/address-pr <N>` | Use PR `#<N>`. |
| `/address-pr only my comments` | Keep threads whose first comment is from the current `gh api user`. |
| `/address-pr @<login>` | Keep threads whose first comment is from `@<login>`. |

Combinations work: `/address-pr 56 @fahchen`.

## Hard rules

1. **Never amend or force-push a pushed commit** unless the user explicitly asks. Review changes are new atomic commits through the `commit` skill.
2. **Never resolve a thread.**
3. **Reply after push, in one batch, after one approval.** `keep` replies wait with `fix` replies.
4. **Match the comment language.** Reply in the language of the comment. Write English replies in ASD-STE100.
5. **Read before you analyse.** Read the cited source at the cited line, and the related code and tests. Do not judge from `diffHunk` alone.
6. **Check ERTS alignment.** When a comment concerns runtime behavior or a runtime model, compare it with the ERTS implementation and observable behavior before you answer. Record any intentional departure in the PR.
7. **Answer questions with evidence.** For BEAM questions, show the `erlc -S` output, the ERTS source concept, or an ERTS run. Do not guess.
8. **Stay in the task scope.** A change outside the PR task goes to the parking lot.
9. **No attribution trailers** in commits or replies.
10. **No manual line-breaks.** Write each reply, decision note, and PR/issue update as one unwrapped paragraph or bullet per line.

## Phases

### Phase 1: Fetch

**1a. Resolve the PR.**

```bash
gh pr view [<N>] --json number,author,headRefName,baseRefName,url,body
```

**1b. Fetch review threads with GraphQL.** Request `databaseId` for the REST reply endpoint:

```bash
gh api graphql -f query='
query($owner:String!, $name:String!, $number:Int!) {
  repository(owner:$owner, name:$name) {
    pullRequest(number:$number) {
      reviewThreads(first:100) {
        nodes {
          isResolved
          isOutdated
          comments(first:50) {
            nodes { databaseId author { login } body path line originalLine diffHunk }
          }
        }
      }
    }
  }
}' -F owner=fahchen -F name=pbeam -F number=<N>
```

Keep threads where `isResolved == false` and the last comment is not a reply from the PR author. Apply the author filter. Keep outdated threads and mark them "outdated". Also read top-level review bodies and PR comments (`gh pr view <N> --json reviews,comments`) and add any that ask for a change.

**1c. Report** the thread count and show the decision table with all rows `open`.

### Phase 2: Discuss one by one

For each thread:

1. Show the file, line, author, comment (verbatim), and the last 8 lines of diff context.
2. Read the cited code and related code.
3. Give your analysis. For a question, answer it; the answer becomes the draft reply. For a change request, assess it, then propose a fix plan or give reasons against it. Name exact code items. Check the knowledge file (`docs/agents/knowledge.md`) and the specification for a rule that applies.
4. Wait for the user.
5. Record `Status`, `Decision`, and the draft reply in the conversation.

When several threads have obvious, independent answers, you can present them together, but each still needs a decision. Do not start Phase 3 before every thread has a decision. Do not post replies.

### Phase 3: Reconcile

Read the whole decision table. Find contradictions, open `discuss` entries, and fixes that change the same lines. Run Phase 2 again for the affected threads until none remain.

### Phase 4: Implement

Skip if every decision is `keep`. Otherwise:

- Read each file before you edit it.
- Make the smallest change that satisfies the decision. Do not change adjacent code.
- Keep the project gates: Rustdoc with `# Examples` on every function, a doc comment on every field, instruction encodings with byte and Erlang examples, 100% application line coverage, no Clippy warnings.
- Run focused checks (`mise run test-focus <name>`, `cargo clippy --all-targets -- -D warnings`).
- When a decision changes the direction, update the affected documents in the same round (`docs/portable-beam-spec.md`, `docs/agents/knowledge.md` through `capture-learnings`, `AGENTS.md`).
- If a fix needs scope expansion, set the thread to `discuss` and return to Phase 2.

Record the `file:line` for each fix.

### Phase 5: Commit

Stage explicit paths only. Invoke the `commit` skill. Make one commit per logical change; several threads can share a commit when they are one change. Run `mise run check-brief` before the last commit of the round. It runs the `mise run check` gate and prints only the result; the full log is in `target/check.log`. Record the short SHA for each `fix`.

### Phase 6: Reply preview

Rewrite each `fix` draft to cite the real commit and location, in past tense. Show all replies at once:

```text
Drafted replies for PR #<N>:

1. `src/runtime/process.rs` #13 - @author [fix]
   `pc` meant program counter. Commit abc1234 renames the field to `next_instruction` and documents it.

2. `src/runtime/process.rs` #28 - @author [keep]
   ERTS has no fixed limit on the number of messages in a mailbox. ...

Push <branch> and post all <N> replies?
```

If every decision is `keep`, ask "Post all <N> replies?".

Responses:

- `yes`, `y`, `push`, `lgtm`: run Phases 7 and 8 without more prompts.
- `edit N: <text>`: replace draft N and show the batch again.
- `skip N`: set thread N to `deferred` and show the batch again.
- Anything else: stop. Do not push or reply.

### Phase 7: Push

```bash
git log --oneline @{upstream}..HEAD
git push
```

If the push fails, show the error, do not retry, and do not reply.

### Phase 8: Reply and update

Post every approved reply in one sweep:

```bash
gh api repos/fahchen/pbeam/pulls/<N>/comments/<first-comment-id>/replies \
  -f body="<reply>" --jq .html_url
```

Then, as `AGENTS.md` requires:

- Update the PR description when a decision, a verification result, or a coverage number changed. Keep the template sections.
- Update the task issue, epic, and affected downstream issues when the scope or direction changed.
- Do not resolve threads.

Wrap up with the final decision table, the reply URLs, deferred threads, parking lot items, and a note that reviewers resolve the threads.

Then offer to capture review learnings for this PR: run `.github/prompts/capture-learnings.prompt.md` with `PR: #<N>`. On an open PR it proposes entries for `docs/agents/knowledge.md` and waits for the user confirmation. A CI workflow runs the same prompt after the PR merges.

## Tips

- `line` is `null` for file-level or deleted-line comments. Show `#- (file-level)` and read the area manually.
- Reply only to the first comment of a thread.
- Quote long non-English comments verbatim.
- More than 100 threads: paginate before filtering.

## What not to do

- Do not write a tracking file or any other file-based tracker.
- Do not commit outside the `commit` skill.
- Do not push before Phase 6 approval.
- Do not ask again between Phases 7 and 8.
- Do not resolve threads.
- Do not reply before the push succeeds.
- Do not post replies one at a time across turns.
- Do not translate replies into another language.
- Do not expand scope in Phase 4.

## Related skills

- `commit`: owns the commit step.
- `draft-pr`: opens the PR.
- `capture-learnings`: promotes a recurring review problem to `docs/agents/knowledge.md`. `.github/prompts/capture-learnings.prompt.md` runs it for the human comments of one PR.
