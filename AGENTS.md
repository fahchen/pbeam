# AGENTS.md

## Project Knowledge

Before implementation, read `docs/agents/knowledge.md`.

## Scope and source of truth

- Read `README.md` and `docs/portable-beam-spec.md` before changing runtime behavior.
- Use GitHub Issues and their dependencies for planning and progress. Do not
  create `tasks/plan.md`, `tasks/todo.md`, or another file-based task tracker.
- Stay within the requested task. Resolve unknowns explicitly; do not invent
  Terra identity, compatibility guarantees, supported platforms, or working commands.

## GitHub hierarchy

- Maintain milestone tracking issue → epic issue → task issue using native
  sub-issue relationships. Implementation PRs link tasks; they are not another
  hierarchy level.
- Every GitHub milestone has a dedicated tracking issue titled `[mN] Title`.
- Epic and task titles use `[mN-eN] Title`. Do not add task-number suffixes;
  GitHub issue numbers identify tasks.
- Assign milestone trackers, epics, and tasks to the corresponding GitHub milestone.
- Keep linked issue bullets in milestone descriptions, milestone tracking issues,
  and epics. Link children back to their epic and milestone tracker.
- Express prerequisites through native blocked-by relationships, with readable
  links in the task body. Keep discovery-driven tasks under their epic rather
  than adding another hierarchy level.

## Labels

- Give each issue exactly one structural label: `kind:milestone`, `kind:epic`,
  or `kind:task`. PRs do not receive structural labels.
- Use existing content labels on tasks and PRs: `documentation` for documentation
  and planning, `enhancement` for new capabilities, and `bug` for defects. Apply
  only labels that describe the actual scope; multiple content labels are allowed.
- Use `status:needs-input` when a concrete unanswered question prevents progress;
  name the missing input in the issue. Use `status:blocked` when an unresolved
  prerequisite prevents work; retain native blocked-by links as the dependency record.
- Status labels describe the issue itself, not every ancestor. Do not propagate
  a child's blocked status to all epics and milestone trackers. Remove status
  labels when resolved; absence of a status label is not proof a task is ready.
- Use GitHub's issue/PR state and review status for open, closed, draft, and review
  lifecycle. Do not add labels for individual milestones, epics, or speculative
  priorities. Keep labels current when scope or blockers change.

## Implementation and review

1. Read the task issue, its epic, milestone tracker, and blockers. Confirm that
   scope, acceptance criteria, and verification are concrete before implementation.
2. Implement only the next ready task. Prerequisite PRs must be reviewed and
   merged before dependent work begins.
3. Deliver one focused task per PR. Split oversized work into scoped issues before
   implementing it; do not build an entire milestone on one branch.
4. Use the PR template. Link the task with `Closes #<issue>`, identify its epic
   and milestone tracker, and link prerequisite PRs.
5. Record decisions made during implementation: the choice, alternatives,
   rationale, tradeoffs, limitations, and supporting evidence. Explain departures
   from the issue/specification and update the affected source. State explicitly
   when no new decisions were necessary.
6. Record exact verification commands and results, including acceptance evidence
   and known gaps. Stop for review after opening the PR; do not auto-merge or
   proceed with dependent implementation.
7. Keep every commit atomic and independently reviewable: make each one a cohesive
   change with its relevant verification, and do not mix unrelated work. Do not
   add functions, fields, or configuration without a current consumer, unless an
   explicit architectural decision records the need, alternatives, and tradeoffs.

Keep affected issues current as work progresses: scope, dependencies, blockers,
decisions, PR links, verification, and remaining work. Add PR links to task issues
and epic PR lists. Synchronize milestone/epic issue bullets after accepted work.
Close tasks only after merge and acceptance verification; close epics and
milestone trackers only after their required children and checkpoints are accepted.
Keep GitHub milestone state consistent with its tracking issue.

Whenever direction or a solution changes, update the affected task bodies,
acceptance criteria, dependencies, milestone/epic checklists, documentation, and
PR description in the same turn. Check downstream tasks for stale assumptions;
replace contradictory current instructions rather than appending a correction.

## Engineering constraints

- Land minimal shared foundations: only the first consumer's required types,
  fields, functions, and configuration. Each later task adds what it needs.
- Reuse existing code, standard libraries, and installed dependencies before
  adding abstractions or dependencies. Logical boundaries in the specification
  do not require pre-created crates.
- Understand the actual flow and all callers before fixing a bug. Fix the shared
  cause and preserve existing supported behavior.
- Verify changed behavior with focused, runnable checks. For documentation-only
  work, check consistency, links, and whitespace; do not scaffold a test framework.
- `mise run check` must pass locally and in CI. Rust formatting and Clippy warnings
  are merge blockers. Require 100% application line coverage; never lower the
  threshold or exclude production code to pass it. An empty crate has no measurable
  coverage and must be reported as N/A, not as 100%.
- Elixir fixtures in `fixtures/hello` use formatting, warnings-as-errors, tests,
  and 100% application coverage in the shared check command. Any additional Elixir
  project must add equivalent gates before introducing behavior.
- Never mutate runtime configuration keys shared with other tests. A uniquely
  test-scoped key may be set inside that test's `setup_all` with a one-line scoping
  comment. Shared keys belong in `config/test.exs` or the language equivalent.
- Distinguish passing fixtures from application compatibility. Record supported
  versions and observed behavior; do not claim unverified OTP or platform coverage.

## Tools and collaboration

- Set development tool versions in `mise.toml`; local and CI setup must use those
  pins rather than duplicating version settings in commands or fixture projects.
- Prefer `fd` and `rg`; use `ast-grep` for structural searches when appropriate.
- Use fish for shell work and `gh` for GitHub operations. Prefer command help
  before external documentation. Use Context7 for library/API documentation and
  setup/code-generation guidance when available.
- Follow the directory's JavaScript lockfile: `bun.lock` means bun;
  `pnpm-lock.yaml` means pnpm.
- Delegate complex research, multi-step investigations, and library documentation
  lookups to subagents. Give editing agents explicit file ownership and preserve
  others' changes.
- Keep communication concise and suited to an experienced systems engineer.
  Write all project documents, GitHub issue and PR text, and assistant messages
  in ASD-STE100 Simplified Technical English. Use short sentences, one instruction
  per sentence, approved words, and consistent terms.
