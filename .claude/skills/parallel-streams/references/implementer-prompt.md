# Implementer prompt template

Copy this file to a scratch file, fill the placeholders, and start each implementer prompt with "Read <that file> first".

OWNER REQUEST (<date>): <milestones> run in parallel streams. Each stream does one task at a time. The owner does not review each PR or each proposal. Write your proposed solution into the issue body (`## Proposed solution`), then code. Merge rule: a Codex or Fable review gives "No findings." and CI is green, and the main session merges. Do not merge. Do not stop for a handback until the PR is open (ready, not draft), the review is done or skipped as below, and CI is green, unless you hit a real blocker.

Read the task issue, its epic and its tracker. Match the pinned ERTS (the OTP version in mise.toml) exactly, and cite the OTP file and line for each behavior in the PR. Add fixture probes against the pinned ERTS. Remove the backlog entries that the issue says this task absorbs, and update the matching docs/erts-differences.md rows. Follow the safe-Rust rule: no new `unsafe`. Other streams change other areas in parallel; avoid unrelated edits, and keep the docs edits small so the merges stay easy.

Use the mise env for cargo (`eval "$(mise env -C <worktree> -s bash)"`, or run `mise trust` in the worktree first); PATH erl may not be the pinned OTP. Run focused checks (`mise run test-focus`, fmt, clippy -D warnings). CI runs the full gate with 100% line coverage. Open the PR with the draft-pr conventions. Review it once with Codex: `codex exec -C <worktree> -m <model> -c model_reasoning_effort="low" -s read-only "Review PR #<n> (use gh pr view and gh pr diff) and its description for correctness against the pinned ERTS and the project rules. Reply 'No findings.' if there are none." < /dev/null`. Fix the findings and run it again until it gives "No findings.". If Codex reports its usage limit, skip it; the main session runs a Fable review. Watch CI with `gh pr checks <n> --watch`. Move the board Status of the task to In Review when the PR opens. Find the board item id with `gh api graphql` on the issue's `projectItems`.

MERGE ORDER: parallel streams negotiate the merge order through the main session. When your PR is ready, or when GitHub reports a conflict, do NOT merge origin/main at once. Report to the main session first: give the PR number, the files you touch (`gh pr diff <n> --name-only`), and whether GitHub reports a conflict. The main session keeps one merge queue, orders the ready PRs so that the fewest branches must merge main again, and tells you when your turn comes. Merge origin/main once at that time, re-run the checks, and push.

TOKEN RULES (also in .claude/agents/pbeam-implementer.md):

- Batch commands. Never Read docs/erts-differences.md or docs/backlog.md whole; use `rg -n` and edit by sentence.
- At about 90 tool calls, push and write "Next steps" in the PR body, then stop.
- Run focused tests only; CI runs the full gate (a bare `cargo test` fails on the Elixir and bundle tests in a fresh worktree).
- Before asking for review, self-check: each OTP citation says what the text says; each PR-body claim has a test or number (no "faster" without a docs/performance record); no field or table without a reader; no code reshaped for coverage; the Linux CI host asserts PBeam's exact output; a dispatch change covers call_ext, apply, apply_last and fun calls; C fixed-width values wrap as in C.

Report the PR number, the remote head SHA from `git ls-remote origin <branch>`, the last review verdict, the CI state and the touched files, in at most 15 lines.
