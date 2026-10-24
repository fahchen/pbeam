# AGENTS.md

## Project Knowledge

Before coding, load project knowledge into context:

1. **Read fully:** `docs/agents/knowledge.md` and `docs/agents/patterns.md` — these are compact and always relevant
2. **Scan titles only:** `docs/agents/improvements.md` — read the `## YYYYMMDD: Title` headings. Read the full entry only when a title is relevant to your current task
3. **Scan frontmatter only:** `docs/agents/postmortems/*.md` — read the YAML `description:` field and file path. Read the full postmortem only when relevant

Write `docs/agents/knowledge.md` and `docs/agents/improvements.md` only through the `capture-learnings` skill. The skill sends a problem class that recurs to `knowledge.md`, and a single wrong turn with a lesson to `improvements.md`. Use `/agent-docs:update-knowledge` only for `patterns.md` and `postmortems/`.

## Scope and source of truth

- Read `README.md` and `docs/portable-beam-spec.md` before changing runtime behavior.
- Use GitHub Issues and their dependencies for planning and progress. Track at-a-glance status on the [project board](https://github.com/users/fahchen/projects/2). Do not create `tasks/plan.md`, `tasks/todo.md`, or another file-based task tracker. The one exception is `docs/backlog.md`: record there each optional improvement and each idea to consider later that has no current consumer or no owner decision, with its reason, evidence, and trigger. When an item gets a consumer or an owner decision, open a task issue for it and remove the item in the PR that closes the issue.
- Stay within the requested task. Resolve unknowns explicitly; do not invent Terra identity, compatibility guarantees, supported platforms, or working commands.

## GitHub hierarchy

- Maintain milestone tracking issue → epic issue → task issue using native sub-issue relationships. Implementation PRs link tasks; they are not another hierarchy level.
- When a task's scope bundles independent mechanisms behind one acceptance gate, split it into sub-issues under that task using native sub-issue relationships. Keep each sub-issue titled `[mN-eN] Title` like a normal task, deliver one focused PR per sub-issue, and record native blocked-by links between sub-issues to express build order. Close the parent task when its sub-issues are accepted and its own acceptance criteria hold.
- Every GitHub milestone has a dedicated tracking issue titled `[mN] Title`.
- Epic and task titles use `[mN-eN] Title`. Do not add task-number suffixes; GitHub issue numbers identify tasks.
- Assign milestone trackers, epics, and tasks to the corresponding GitHub milestone.
- Keep linked issue bullets in milestone descriptions, milestone tracking issues, and epics. Link children back to their epic and milestone tracker.
- Express prerequisites through native blocked-by relationships, with readable links in the task body. Keep discovery-driven tasks under their epic rather than adding another hierarchy level.

## Labels

- Give each issue exactly one structural label: `kind:milestone`, `kind:epic`, or `kind:task`. PRs do not receive structural labels.
- Use existing content labels on tasks and PRs: `documentation` for documentation and planning, `enhancement` for new capabilities, and `bug` for defects. Apply only labels that describe the actual scope; multiple content labels are allowed.
- Use `status:needs-input` when a concrete unanswered question prevents progress; name the missing input in the issue. Use `status:blocked` when an unresolved prerequisite prevents work; retain native blocked-by links as the dependency record.
- Status labels describe the issue itself, not every ancestor. Do not propagate a child's blocked status to all epics and milestone trackers. Remove status labels when resolved; absence of a status label is not proof a task is ready.
- Use GitHub's issue/PR state and review status for open, closed, draft, and review lifecycle. Do not add labels for individual milestones, epics, or speculative priorities. Keep labels current when scope or blockers change.

## Project board

- Track at-a-glance status on the [project board](https://github.com/users/fahchen/projects/2), not a new file-based tracker.
- The board's `Status` field tracks the work lifecycle: `Todo` (not started), `Designing` (drafting a proposed solution/approach), `Pending Review` (proposal awaiting confirmation before coding starts), `In Progress` (coding), `In Review` (PR open, awaiting review/approval), `Done` (merged/closed). `status:blocked` stays a label, not a `Status` value.
- Set `Status` the moment it changes, not in a later batch:
  - `Designing` when you start drafting a proposed solution.
  - `Pending Review` when the proposal is ready and you stop for confirmation.
  - `In Progress` the moment confirmation arrives and coding starts.
  - `In Review` the moment the PR opens.
  - `Done` the moment the PR merges and acceptance is verified.
  Every PR, merge, and status-label change that touches an issue updates that issue's board `Status` in the same turn.

## Implementation and review

1. Read the task issue, its epic, milestone tracker, and blockers. Confirm that scope, acceptance criteria, and verification are concrete before implementation.
2. Implement only the next ready task. Prerequisite PRs must be reviewed and merged before dependent work begins.
3. Deliver one focused task per PR. Split oversized work into scoped issues before implementing it; do not build an entire milestone on one branch.
4. Use the PR template. Link the task with `Closes #<issue>`, identify its epic and milestone tracker, and link prerequisite PRs.
5. Record decisions made during implementation: the choice, alternatives, rationale, tradeoffs, limitations, and supporting evidence. Explain departures from the issue/specification and update the affected source. State explicitly when no new decisions were necessary.
6. Record exact verification commands and results, including acceptance evidence and known gaps. Stop for review after opening the PR; do not auto-merge or proceed with dependent implementation.
7. Keep every commit atomic and independently reviewable: make each one a cohesive change with its relevant verification, and do not mix unrelated work. Do not add functions, fields, or configuration without a current consumer, unless an explicit architectural decision records the need, alternatives, and tradeoffs.
8. Add one file to `docs/performance/` in the same pull request that lands a measured optimization. `docs/performance-status.md` gives the name, the content, and the rules of the file.

Keep affected issues current as work progresses: scope, dependencies, blockers, decisions, PR links, verification, and remaining work. Add PR links to task issues and epic PR lists. Synchronize milestone/epic issue bullets after accepted work. Close tasks only after merge and acceptance verification; close epics and milestone trackers only after their required children and checkpoints are accepted. Keep GitHub milestone state consistent with its tracking issue.

Whenever direction or a solution changes, update the affected task bodies, acceptance criteria, dependencies, milestone/epic checklists, documentation, and PR description in the same turn. Check downstream tasks for stale assumptions; replace contradictory current instructions rather than appending a correction.

## Engineering constraints

- Align runtime behavior with the pinned ERTS, which is the OTP version in `mise.toml`. Before you add or change an instruction, BIF, or term operation, read its pinned OTP source and run it on the pinned ERTS. Match the observable result exactly: return value, exception class and reason, messages sent, and side effects. Run the pinned OTP module code; do not replace it with a Rust shortcut. Do not add a limit or fast path that ERTS does not have. Cite the OTP source file and line for each such decision in the PR. A part that is not implemented yet fails with a clear runtime error, and `docs/erts-differences.md` records it; never ship different behavior as a workaround.
- Do not introduce `unsafe`. New code stays in safe Rust, and no change adds an `unsafe` block, an `unsafe impl`, a raw pointer, or an unchecked access that the language cannot prove safe. The constraint applies to new code and to the runtime, term, interpreter, and loader paths. The 104 pre-existing `unsafe` uses are host FFI calls in `src/runtime/host.rs`, `src/runtime/host/` (with the 14 of the `inet_gethost` host FFI in `src/runtime/host/gethost.rs`, the 8 of the owner-approved time FFI of #1183 in `src/runtime/host/clock.rs`, the 8 of the owner-approved network database FFI of #1219 in `src/runtime/host/netdb.rs`, the 5 of the owner-approved interface address FFI of #1254 in `src/runtime/host/ifaddrs.rs`, and the owner-approved `envp` raw pointer array and `posix_spawn_file_actions_addchdir_np` call of #1203 inside the existing `posix_spawn` block of `src/runtime/host/spawn.rs`), the 2 owner-approved `setsockopt` and `getsockopt` calls of #1209 in `src/runtime/interpreter/native/prim_socket/socket/options.rs` (#1209 options.rs: `raw_set` and `raw_get`, in place of `nix::sockopt_impl!`), and the PCRE2 binding `src/runtime/pcre2.rs`, including the owner-approved name-table FFI of `Code::names` that calls the `pbeam_pcre2_name_table` shim in `src/runtime/pcre2_serialize_guard.c`, plus the static atom name list in `src/term/atom/static_names.rs`; they are pre-existing and out of scope. An exception exists for the NIF ABI layer: `docs/adr/0001-unsafe-in-the-nif-abi-layer.md`.
- Land minimal shared foundations: only the first consumer's required types, fields, functions, and configuration. Each later task adds what it needs.
- Reuse existing code, standard libraries, and installed dependencies before adding abstractions or dependencies. Logical boundaries in the specification do not require pre-created crates.
- Understand the actual flow and all callers before fixing a bug. Fix the shared cause and preserve existing supported behavior.
- Verify changed behavior with focused, runnable checks. For documentation-only or skill-only work, run `mise run lint` (it includes `rumdl check .`); do not scaffold a test framework.
- Cover each new branch with a fixture probe against the pinned ERTS. Use a unit test only for a path that a compiled fixture cannot build, such as a bitstring that is not a whole number of bytes. Remove a branch that cannot run instead of leaving it without a test.
- Put each test in `tests/`; `src/` holds no test module. A test-only helper that needs private fields of a type, such as `Mailbox::messages`, can stay in `src/` under `#[cfg(test)]`. The unit tests of a module go in `tests/<module path without src/>.rs`, and the module loads them with `#[cfg(test)] #[path = "..."] mod tests;`, so they keep access to private items. A test that runs the `pbeam` binary or a mise task goes in `tests/feature/`. A test that drives the library API with compiled fixtures goes in `tests/integration/`. `Cargo.toml` sets `autotests = false`, so add a `[[test]]` target for each new feature or integration test file. Shared helpers stay in `tests/common/`.
- Run the necessary lightweight checks while you work: `mise run test-focus`, `cargo fmt --all -- --check`, Clippy, and `mise run lint` for documentation-only changes. CI runs the gate of the change on the PR head, and it is the final gate: `scripts/ci_task.sh` selects the full `mise run check` for a code change and `mise run lint` for a documentation-only change. Run `mise run check` locally only when a change needs evidence that CI cannot give, such as a platform-specific failure, and record why. Rust formatting and Clippy warnings are merge blockers. Require 100% application line coverage; never lower the threshold or exclude production code to pass it. An empty crate has no measurable coverage and must be reported as N/A, not as 100%.
- Elixir fixtures in `fixtures/hello` use formatting, warnings-as-errors, tests, and 100% application coverage in the shared check command. Any additional Elixir project must add equivalent gates before introducing behavior.
- Never mutate runtime configuration keys shared with other tests. A uniquely test-scoped key may be set inside that test's `setup_all` with a one-line scoping comment. Shared keys belong in `config/test.exs` or the language equivalent.
- Distinguish passing fixtures from application compatibility. Record supported versions and observed behavior; do not claim unverified OTP or platform coverage.

## Tools and collaboration

- Set development tool versions in `mise.toml`; local and CI setup must use those pins rather than duplicating version settings in commands or fixture projects.
- Use these compact `mise` tasks in development loops. They print results and failures, not full logs. `mise run check` stays the merge gate. `mise run <task> --help` shows the arguments.
  - `mise run check-brief`: run `mise run check`. Print the result and the coverage line, or the failure. The full log is in `target/check.log`. When the only failure is the known `erl_child_setup: failed with error 32` failure of the pty test `restores_the_terminal_before_the_result_of_terra_counter_as_interactive_erts`, the task runs that test once more alone, checks the coverage again, and prints `check: retried <test>`.
  - `mise run test-focus [cargo test arguments] [-- <harness arguments>]`: run selected Rust tests, for example `--test order -- --nocapture`. `--test <name>` selects one target of `tests/feature/` or `tests/integration/`. `--lib` selects the unit tests.
  - `mise run erts-eval [--pa <dir>] '<expressions>'`: evaluate on the pinned OTP as `erl -eval` does. Print the value or `exception <class>: <reason>`. The evaluating process traps exits, as with `erl -eval`. Exit signals arrive as `{'EXIT', From, Reason}` messages. Only `exit(self(), kill)` stops the runtime.
  - `mise run disasm <module | file.beam> [name/arity]...`: print the `beam_disasm` instructions without `{line,_}`.
  - `mise run fixture-beam [--write] [fixtures/<dir>/<name>.erl]...`: compile the Erlang fixtures and compare them with the committed `.beam` files, except `CInf` and `Dbgi`.
  - `mise run otp-src <path> <start> <end>`: print numbered lines of an OTP source file at the pinned `erlang` tag, from the git repository in `$PBEAM_OTP_SRC` (default `~/.cache/pbeam/otp`).
  - `mise run issue <number> [section]...`: print the title, state, labels, milestone, the text before the first section, and each `#` or `##` section whose heading starts with a given name. With no name, print the headings. The output omits blank lines and other sections, so never use it as the body of `gh issue edit`.
- Prefer `fd` and `rg`; use `ast-grep` for structural searches when appropriate.
- Use the Serena MCP server for code navigation. `.serena/project.yml` configures the `rust` and `elixir` language servers, and each worktree finds it through `--project-from-cwd`.
  - Prefer Serena (`find_symbol`, `get_symbols_overview`, `find_referencing_symbols`) for symbol discovery, definitions, references, and semantic navigation.
  - Use `rg` and `fd` for plain text searches and for locating files.
  - Do not read a whole file when symbol-level retrieval is enough. Read only the symbol bodies that the task needs.
  - Use the smallest context that the task needs.
  - Do not use Serena where one simple shell command is faster, such as listing files, counting matches, or reading a short file.
  - Fall back to `rg`, `ast-grep`, and file reads when Serena cannot resolve a symbol or does not support the language.
- Use fish for shell work and `gh` for GitHub operations. Prefer command help before external documentation. Use Context7 for library/API documentation and setup/code-generation guidance when available.
- Follow the directory's JavaScript lockfile: `bun.lock` means bun; `pnpm-lock.yaml` means pnpm.
- Delegate complex research, multi-step investigations, and library documentation lookups to subagents. Give editing agents explicit file ownership and preserve others' changes.
- Keep communication concise and suited to an experienced systems engineer. Write all project documents, GitHub issue and PR text, and assistant messages in ASD-STE100 Simplified Technical English. Use short sentences, one instruction per sentence, approved words, and consistent terms.
- Do not add manual line-breaks inside a paragraph or bullet in Markdown files, GitHub issues, or PRs. Write each paragraph or bullet as one unwrapped line; let the renderer or editor soft-wrap it. Manual line-breaks mid-sentence cause ragged diffs on edit and fight soft-wrap in the GitHub UI.
