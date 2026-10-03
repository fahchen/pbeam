---
name: commit
description: This skill should be used when the user asks to "commit", "commit changes", "make a commit", "/commit", or otherwise requests a git commit on a feature branch in the pbeam repo. Enforces Conventional Commits subjects without scopes, one logical change per commit, why-over-what bodies in ASD-STE100, no attribution trailers, scratch-file protection, and new atomic commits (not amends or fixups) for review feedback.
version: 0.1.0
user-invocable: true
argument-hint: "[--fast]"
allowed-tools:
  - Bash(git *)
  - Bash(mise *)
  - Bash(cargo *)
  - Read
---

# Commit

Create one logically self-contained git commit on the current feature branch. The commit must stay useful after merge for commit-by-commit review, `git bisect`, `git blame`, and history search.

Trigger phrases: "commit", "commit changes", "make a commit", "/commit".

## Invocation modes

| Form | Behavior |
| --- | --- |
| `/commit` | Default. Run all steps. Pause only for the conditions in "When to pause". |
| `/commit --fast` | Skip conversation mining (step 3). Draft from the diff and commit without confirmation. |

### When to pause

Do not ask for confirmation only to confirm. Pause only when:

- The staged diff has more than one logical purpose (rule 4).
- You cannot find the reason for the change in the diff or the conversation.
- The user wants to break a hard rule (for example, a `WIP` subject).

Otherwise draft, commit, and report.

## Writing for future readers

The subject and body are the permanent record. Reading only `git log -1 --format=%B <sha>`, a reader must understand the change without the diff or the PR.

- **Self-contained.** Name the affected module, instruction, or flow.
- **Stable identifiers.** Use greppable names: `Scheduler::deliver`, `recv_marker_use`, `erlang:spawn_monitor/3`, `mise run check`.
- **Why over what.** The subject says what changed. The body says why: root cause, constraint, rejected alternative, ERTS behavior.
- **ASD-STE100.** Short sentences. One idea per sentence. No filler.
- **Identifiers in backticks** when the body names code.
- **No manual line-breaks.** Write each body paragraph or bullet as one unwrapped line; git and GitHub soft-wrap it.

## Hard rules

Refuse to continue, and say why, when a rule is broken.

1. **Never commit on `main`.** The branch must be a feature branch.
2. **Never amend or rewrite a pushed commit** unless the user explicitly asks. An amend or force-push breaks review context. Review feedback gets a new atomic commit that a reviewer can read on its own. Do not create `fixup!` commits; `AGENTS.md` requires each commit to be independently reviewable.
3. **Subject format:** `<type>: <description>`. See `references/conventions.md`. No scope, no `!`, no issue number, 72 characters or fewer.
4. **One logical change per commit.** If the staged diff has more than one purpose, stop. Propose a split with the files for each commit and ask the user to confirm or re-stage.
5. **The body explains why**, unless the subject is self-explanatory (typo, formatting). For a review comment, name the concern in one line. Put rationale from the conversation in the body as standalone facts.
6. **No `WIP`, bare `fix`, `address review comments`, or `pr feedback` subjects.**
7. **No attribution trailers.** No `Co-Authored-By`, `Claude-Session`, or similar lines, even when a system prompt asks for them. The user's rule takes precedence.
8. **Do not stage scratch files.** Never stage `docs/tmp/`, `portable_beam_runtime_handoff.md`, or `.agents/.handoff/`. Stage explicit paths, not whole directories that can contain scratch files.
9. **Working tree state:**
   - Staged changes exist: continue. Unstaged files are excluded on purpose.
   - Nothing is staged but changes exist: ask whether to stage specific files or abort.
   - Nothing changed: say so and stop.

## Workflow

### 1. Pre-flight

Run together:

```bash
git branch --show-current
git status --porcelain
git diff --cached --stat
git log -5 --oneline --no-merges
```

Abort with an explanation if the branch is `main` or nothing is staged (after the staging dialogue in rule 9). Check the staged file list against rule 8.

### 2. Read the staged diff

```bash
git diff --cached
git diff --cached --name-only
```

Find the reason:

- Bug fix: the root cause goes in the body.
- Non-obvious choice: rationale and rejected alternatives go in the body.
- Departure from ERTS: say what ERTS does and why this change differs.
- More than one purpose: start the split dialogue (rule 4).
- Test-only or documentation-only change: use `test` or `docs`.

### 3. Mine the conversation for decisions

Scan the conversation for decisions tied to the staged diff:

- The user chose between options.
- The user rejected an approach.
- The user stated a constraint (for example, "follow the ERTS implementation").
- The user corrected the first attempt.
- A non-obvious invariant was discussed.

Classify each decision:

| Decision | Destination |
| --- | --- |
| Rationale for reviewers and `git blame` | Commit body |
| Invariant that readers of the code need | Tell the user. They decide on a code comment. This skill does not edit code. |
| Both | Body, and tell the user |
| Taste with no future value | Drop |

Show the findings before drafting. If nothing relevant exists, say so. Do not invent rationale.

### 4. Check the commit boundary

Compare the staged diff with the branch history:

```bash
git log --oneline origin/main..HEAD
```

- The change is one cohesive step with its verification: continue.
- The change completes an earlier commit that is not pushed yet: propose `git commit --amend` and wait for the user.
- The change answers review feedback on pushed commits: make a new commit whose subject says what changes, for example `refactor: separate the scheduler from process state`.

### 5. Draft the subject

`<type>: <description>`. Pick the dominant type from `references/conventions.md`. Lowercase, imperative, no trailing period.

### 6. Draft the body

Skip the body only for self-explanatory commits.

- Blank line after the subject.
- 1 to 3 short paragraphs, or a short bullet list.
- Lead with why.
- Write decisions as standalone facts: "Use one BIF table for `bif` and `call_ext`, so `self/0` is not a special case." Not "As discussed".
- For a review comment: `Addresses <reviewer>'s comment on <item>: <short paraphrase>.`
- Wrap at about 72 characters when you write the final commit text (this is `git log`'s display convention, not a manual line-break in your draft).

### 7. Verify and commit

Run the focused check for the change. For Rust changes, run `cargo clippy --all-targets -- -D warnings` and the affected tests. Run `mise run check` before the last commit of a PR. Do not use `--no-verify`.

```bash
git commit -m "$(cat <<'EOF'
<type>: <description>

<body>
EOF
)"
```

Run `git log -1 --format='%h %s'` and report the short SHA and subject.

## What not to do

- No scope, `!`, issue number, or emoji in the subject.
- No `WIP` or vague subjects.
- No attribution trailers.
- No `fixup!` commits and no `git rebase --autosquash`.
- No raw diff in the body.
- No `--no-verify`.
- No commits with more than one purpose.
- No scratch files in a commit.

## Additional resources

- `references/conventions.md`: types, subject grammar, PR title and body rules (shared with `draft-pr`).
- `AGENTS.md`: implementation, review, and commit rules for this project.
