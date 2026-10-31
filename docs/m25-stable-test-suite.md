# M25: Stable test suite

## Goal and scope

The full unit, integration, and feature test suite must pass under parallel load on macOS arm64 and Ubuntu. No test binary may crash. This is the follow-on milestone for the test failures found during the completed BeautifulRust work. GitHub already tracks it as [m25](https://github.com/fahchen/pbeam/milestone/25), with [tracker #1440](https://github.com/fahchen/pbeam/issues/1440) and [epic #1441](https://github.com/fahchen/pbeam/issues/1441). This repository document gives another agent the goal, accepted constraints, verified progress, and next steps.

[#1442: stop the macOS SIGPIPE test crash](https://github.com/fahchen/pbeam/issues/1442) has a fix in draft [PR #1475](https://github.com/fahchen/pbeam/pull/1475). After that PR merges, select the next ready task from #1443, #1444, and #1473. Reproduce each failure and establish its cause before changing code. Split a task if its failures have independent causes. Keep one focused task per PR.

This milestone does not reopen the completed lint rollout, restore blaze, or take ownership of the m24 distribution and Redbug plan. The completed work below supplies evidence and context.

## Completion gate

- #1442 passes on macOS arm64 and Ubuntu without a test binary crash.
- #1443 identifies both causes and passes 20 full parallel unit test runs with no `SIGSEGV` stop of the test binary. #1473 identifies its cause and passes 20 full parallel unit test runs.
- #1444 identifies both causes and passes 20 parallel runs of each affected target.
- Each runtime fix matches pinned ERTS behavior and cites the relevant OTP source. No new unsafe operation or shared test configuration mutation is introduced.
- Each task has accepted verification and a merged PR. Formatting, Clippy, focused tests, and CI's full gate pass, with 100% application line coverage.
- The task issues, epic, tracker, milestone state, and project Status agree after acceptance. Close the epic and milestone only when all required children and the milestone checkpoint are accepted.

## Progress summary

| Area | Completed | Remaining |
| --- | --- | --- |
| Milestone planning | m25 tracker, epic, native task relationships, and project items exist. All four tasks have acceptance criteria. | Recheck scope and dependencies before selecting each task. |
| Runtime and test fixes | #1442 has a fix in draft PR #1475. Its cause is the `SIG_DFL` reset of a failed `Command::exec` in the embedded unit tests, which leaves a port waiter write to kill the test binary with `SIGPIPE`. | #1442 awaits Ubuntu CI and review. #1443, #1444, and #1473 are Open / Todo. |
| Supporting work | m23 lint rollout, Linux Clippy helper, benchmark comparison, and requested blaze removal are delivered. | Optional old-branch cleanup is separate from milestone acceptance. |

On 2026-10-31, the `gh` token of the working session had no `read:project` scope, so this session could not set the project Status of #1442 to `In Review`. A session with project scope must set it. GitHub issues and the [PBeam project board](https://github.com/users/fahchen/projects/2) remain the source of current scope and status. This document is a dated milestone specification and handoff, not a second task tracker. Update GitHub as work progresses. Update this document when the goal, decisions, acceptance criteria, or next step changes.

## Tasks and current progress

The [m25 tracker #1440](https://github.com/fahchen/pbeam/issues/1440), [epic #1441](https://github.com/fahchen/pbeam/issues/1441), and milestone remain open. The tracker and epic have project Status `Todo`. The epic now lists #1473 and draft PR #1475. Its task bodies state no blocker and independent work, with #1442 first because the crash stopped the macOS unit test binary. Recheck native dependencies and active PRs before selecting work.

| Task | Issue / project Status | Failure evidence | Required acceptance |
| --- | --- | --- | --- |
| [#1442](https://github.com/fahchen/pbeam/issues/1442) | Open, draft PR #1475, project Status not updated | On macOS arm64, `reads_an_empty_output_pipe_without_a_block` stopped the unit test binary with SIGPIPE, exit 141, including on unchanged main. The cause is the `SIG_DFL` reset of a failed `Command::exec` in the embedded unit tests (`library/std/src/sys/process/unix/unix.rs` in Rust 1.99.0, `prepare_sigpipe`). A port waiter write to a closed status pipe then stopped the binary. | Pass on macOS arm64 and Ubuntu without a binary crash. Determine whether the cause belongs to the runtime or test. Cite pinned OTP source if runtime behavior changes. Ubuntu CI and review are pending. |
| [#1443](https://github.com/fahchen/pbeam/issues/1443) | Open / Todo | Environment and release-work unit tests failed in parallel, including on unchanged code, and passed alone or serially. The same runs also stop the binary with `SIGSEGV` in about 1 of 3 runs. Recorded hypothesis: `host::environment` reads `environ` without the std environment lock while an embedded unit test runs `Command::exec`, which replaces `environ` with a temporary array. | Find both causes and the `SIGSEGV` cause. Pass 20 full parallel unit test runs with no binary crash. Preserve test configuration isolation. Split into sub-issues if the causes differ. |
| [#1444](https://github.com/fahchen/pbeam/issues/1444) | Open / Todo | `nif_resources` failed in 3 of 6 old-main runs: expected `[5,6,5]`, observed `[]`. An Ubuntu `init_boot` run observed `num_dinet => 1` where ERTS returned `0`. | Find both causes, with OTP evidence for runtime changes. Pass 20 parallel runs of each target. Split into sub-issues if the causes differ. |
| [#1473](https://github.com/fahchen/pbeam/issues/1473) | Open / Todo | Added after the source session. Linux CI observed a late reply from a cancelled gethost request; the same-commit rerun and 15 isolated macOS runs passed. | Find the cause, with pinned OTP evidence if runtime behavior is the cause. Pass 20 full parallel unit test runs. |

The race and delayed-close explanations in these issues are hypotheses. Do not treat them as established causes. Shared acceptance includes focused checks, formatting, Clippy, and CI's full `mise run check` with 100% application line coverage. Do not replace the required parallel runs with isolated passes or lower the coverage threshold.

Current source entry points:

- #1442: `tests/runtime/host/spawn.rs`, `reads_an_empty_output_pipe_without_a_block`; runtime code in `src/runtime/host/spawn.rs`.
- #1443: `tests/runtime/host/environment.rs`, `changes_its_own_copy_and_not_the_environment_of_the_process`; `tests/runtime/interpreter.rs`, `drains_the_release_work_of_an_exited_process_while_the_root_waits`.
- #1473: `tests/runtime/host/gethost.rs`, `cancels_a_request_of_the_global_queue_or_a_worker_queue`; runtime code in `src/runtime/host/gethost.rs`.
- #1444: Cargo targets `nif_resources` and `init_boot`; `tests/integration/nif_resources.rs` and `tests/integration/init_boot.rs`.

## Resume procedure

1. Read `AGENTS.md`, `docs/agents/knowledge.md`, and `docs/agents/patterns.md`. Scan the relevant improvement entries. Read `README.md` and `docs/portable-beam-spec.md` before a runtime change.
2. Refresh #1440, #1441, #1442–#1444, and #1473. List the native child issues to find later additions. Inspect native dependencies, linked PRs, and project Status. Review draft PR #1475 first. Then prefer #1443 unless it is already assigned, completed, or superseded.
3. Set the selected task to `Designing` when drafting its solution. Reproduce the failure on the current revision and distinguish test behavior from runtime behavior. Record evidence in the issue.
4. For runtime changes, read the pinned OTP source and run the pinned ERTS probe before selecting the fix. Current pins are Rust 1.99.0 and OTP 29.1.1; `mise.toml` is authoritative.
5. Follow the task's acceptance criteria and the proposal confirmation flow. Set `Pending Review` for a proposal awaiting confirmation and `In Progress` when approved coding starts. Deliver one focused task PR. Keep shared runtime configuration keys unchanged.
6. Run the necessary focused checks. CI supplies the full gate. Record revision, platform, exact commands, results, and remaining gaps. Set `In Review` when the PR opens. Close the task and set `Done` only after merge and acceptance verification. Keep the epic and tracker current.

Useful refresh and focused-check commands, from the repository root:

```sh
gh issue view 1440 --repo fahchen/pbeam
gh issue view 1441 --repo fahchen/pbeam
gh issue view 1442 --repo fahchen/pbeam --json number,title,state,body,projectItems,closedByPullRequestsReferences
gh issue view 1443 --repo fahchen/pbeam --json number,title,state,body,projectItems,closedByPullRequestsReferences
gh issue view 1444 --repo fahchen/pbeam --json number,title,state,body,projectItems,closedByPullRequestsReferences
gh issue view 1473 --repo fahchen/pbeam --json number,title,state,body,projectItems,closedByPullRequestsReferences
gh api repos/fahchen/pbeam/issues/1441/sub_issues
mise run test-focus --lib reads_an_empty_output_pipe_without_a_block
mise run test-focus --test nif_resources
mise run test-focus --test init_boot
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

These focused commands are starting points. They do not replace #1443's 20 full parallel unit test runs or #1444's 20 parallel runs of each target. Refresh the relevant issue before execution. No m25 implementation, branch deletion, or blaze restoration is authorized merely by reading this document.

## Inherited goals and owner decisions

| Goal or decision | Required result | Current result |
| --- | --- | --- |
| Install [BeautifulRust](https://github.com/morganlinton/BeautifulRust) in PBeam | Install the Claude plugin at project scope, not user scope. Apply its full lint configuration and fix the hits. | Delivered by #1353 and #1384. m23 is closed. |
| Fix by lint type | Use `cargo clippy --fix` or ast-grep for mechanical changes. Use Sonnet agents for the remaining groups. Preserve runtime behavior. | Delivered. The task and PR map is below. |
| Preserve the unsafe constraint | Add no unsafe operation. Keep the existing approved exceptions. Leave `multiple_unsafe_ops_per_block` as `allow`; the owner explicitly declined block splitting. | Recorded in #1342 and #1348. Preserve this decision. |
| Permit a worktree and stacked work for m23 | These were owner exceptions for this work. Later, combine the stack into one PR before merging to main. | #1384 combines the stack. The temporary m23 worktree was removed. These exceptions do not authorize later work. |
| Check Linux-only code locally | Provide a Linux environment for Clippy before CI. | The `pbeam-linux` OrbStack machine exists. Its limits are below. |
| Track test failures found during m23 | Open tasks with reproduction evidence and acceptance criteria. | m25 tasks #1442–#1444 exist. Implementation remains open. |
| Review and fix benchmark PR #1431 | Make the PBeam and pinned ERTS timings comparable. Make direct script use select the pinned tools. | Fixed and merged. Task #1430 is Done. |
| Review and merge blaze PR #1389, then remove it | The later owner instruction removes the integration because it cannot load this workspace and gives no build or check benefit. | #1457 removes it. Reconsider only at the backlog trigger. |

The session did not own the full m24 distribution and Redbug plan. It reviewed benchmark #1431 from that concurrent stream. Do not infer an instruction to implement all m24 from this milestone document.

## Completed supporting work and project status

### m23: Idiomatic Rust lints

The [m23 tracker #1342](https://github.com/fahchen/pbeam/issues/1342), [epic #1343](https://github.com/fahchen/pbeam/issues/1343), and GitHub milestone are closed. Both issues and every task below have project Status `Done`.

| Task | Scope | Delivery | Issue / project Status |
| --- | --- | --- | --- |
| [#1344](https://github.com/fahchen/pbeam/issues/1344) | Lint and plugin foundation | [#1353](https://github.com/fahchen/pbeam/pull/1353) | Closed / Done |
| [#1345](https://github.com/fahchen/pbeam/issues/1345) | Small pedantic style lints | #1384, replaces #1356 | Closed / Done |
| [#1346](https://github.com/fahchen/pbeam/issues/1346) | Visibility and imports | #1384, replaces #1357 | Closed / Done |
| [#1347](https://github.com/fahchen/pbeam/issues/1347) | Documentation lints | #1384, replaces #1358 | Closed / Done |
| [#1348](https://github.com/fahchen/pbeam/issues/1348) | Unsafe block lints | #1384, replaces #1360 | Closed / Done |
| [#1349](https://github.com/fahchen/pbeam/issues/1349) | API shape lints | #1384, replaces #1369 | Closed / Done |
| [#1350](https://github.com/fahchen/pbeam/issues/1350) | Unwrap and panic lints | #1384, replaces #1370 | Closed / Done |
| [#1351](https://github.com/fahchen/pbeam/issues/1351) | Cast parent task | #1384, through its children below | Closed / Done |
| [#1371](https://github.com/fahchen/pbeam/issues/1371) | Casts in term and beam | #1384, replaces #1377 | Closed / Done |
| [#1372](https://github.com/fahchen/pbeam/issues/1372) | Casts in binary, erlang, and ETS BIFs | #1384, replaces #1378 | Closed / Done |
| [#1373](https://github.com/fahchen/pbeam/issues/1373) | Casts in other BIFs and the interpreter | #1384, replaces #1379 | Closed / Done |
| [#1374](https://github.com/fahchen/pbeam/issues/1374) | Casts in native functions and NIFs | #1384, replaces #1381 | Closed / Done |
| [#1375](https://github.com/fahchen/pbeam/issues/1375) | Remaining casts and lint activation | #1384, replaces #1383 | Closed / Done |
| [#1352](https://github.com/fahchen/pbeam/issues/1352) | Suppression and output lints | #1384 | Closed / Done |

Foundation PR #1353 merged as `6ed35134`. [Combined PR #1384](https://github.com/fahchen/pbeam/pull/1384) merged as `e45cc1ee`. The eleven intermediate PRs were closed and replaced, not individually merged. Their decisions remain in those PRs.

The final #1384 [CI run 40980513974](https://github.com/fahchen/pbeam/actions/runs/40980513974) has a successful `checks` result. The source session reports 2,372 passing tests, 100% application line coverage, and clean full-diff and final-delta Codex reviews for that delivery. Earlier macOS failures are the evidence for m25, not proof that current main passes every local test.

Keep the explicit `#[expect(..., reason = "...")]` decisions where runtime semantics or a shared signature require them. Do not hide a lint through a type alias. One cast fix preserved the ETS trapped writes counter: signed `-1` represents `u64::MAX`, so the conversion uses `cast_unsigned()` rather than a checked conversion that panics. The combined PR records this regression and fix.

Issue #1351 still has unchecked child and acceptance bullets in its body. Its children are all Closed / Done, and #1384 delivered the parent. Treat those bullets as stale historical text, not unfinished cast work. This handoff does not edit that issue body.

### Benchmark comparison

[Task #1430](https://github.com/fahchen/pbeam/issues/1430) is Closed / Done. [PR #1431](https://github.com/fahchen/pbeam/pull/1431) merged as `0321911f` after fixes and review on head `4ae65e6d`.

- `benchmark/bench_run.erl` times both runtimes inside the guest with `erlang:monotonic_time(nanosecond)`. Runtime and root-process setup and teardown are outside the measured work.
- The same compiled scenarios run on both runtimes. The harness checks equal results for arithmetic, spawn and message exchange, lists, maps, and ETS.
- `benchmark/run.sh` runs tools through `mise exec`, including when invoked directly. The documentation describes five scenarios and the integration test rather than the removed example.
- The session records passing debug and release benchmark tests, CI, and a clean review of the final head. See [the benchmark method and limits](../benchmark/README.md) before using a ratio as performance evidence.

The related [epic #1398](https://github.com/fahchen/pbeam/issues/1398) and [m24 tracker #1390](https://github.com/fahchen/pbeam/issues/1390) remain Open / Todo. [Task #1429](https://github.com/fahchen/pbeam/issues/1429) remains Open / Todo. It targets a Terra 200×50 frame at a median of at most 86 ms, from its recorded 172 ms baseline. It runs only when no other m24 task is ready and blocks no task. It requires a performance spike and an interleaved before and after measurement. Benchmark delivery does not satisfy that optimization task.

### blaze experiment and removal

[PR #1389](https://github.com/fahchen/pbeam/pull/1389) merged as `152563d2`. Its review added installer SHA-256 validation. [PR #1457](https://github.com/fahchen/pbeam/pull/1457) then merged as `878d2912` and removed the workflow, version pin, installer script, mise task, and blaze document at the owner's request.

The measured release could not load the C dependency build scripts because of a jobserver helper's writable static without a rustc symbol. The informational CI job gave no working build or check. The [Build with blaze backlog entry](backlog.md#build-with-blaze) retains the evidence and return condition: an upstream release must load this workspace and agree with cargo. The old implementation is available at `152563d2`. Do not restore blaze as a missing deliverable.

## Local Linux helper and cleanup

On 2026-10-31, `orb list` shows `pbeam-linux` running Ubuntu on arm64. The source session installed Rust, Clippy, rustfmt, and C build tools. Its Linux Clippy run on #1384 head `4b763420` passed in 52 seconds. It did not install OTP, Elixir, or coverage tools. Their current availability was not checked. This machine does not prove x86_64-specific behavior or replace Ubuntu CI.

The historical Clippy command below uses the current checkout instead of the deleted m23 worktree. Check the Rust version against `mise.toml` before use. The Linux target directory must be separate from the macOS build output. This command was not rerun for the handoff.

```sh
orb -m pbeam-linux bash -lc '. ~/.cargo/env; cd /Users/fahchen/PersonalProjects/pbeam && CARGO_TARGET_DIR=$HOME/pbeam-target cargo clippy --locked --workspace --all-targets --all-features -- -D warnings'
```

The source session removed the m23 worktree, local m23 branches, and temporary build cache. The m23 worktree is absent from the current `.claude/worktrees/` listing. Do not reuse its old paths.

Eleven remote `m23-e1-*` branches still exist, verified with `git ls-remote --heads origin 'm23-e1-*'`. Their PRs are closed and replaced by #1384. The earlier session's permission check rejected remote branch deletion. No deletion is part of this documentation change. This optional cleanup does not block m25. The old blaze Actions cache was also left alone; its current existence and size were not checked.

## Sources and verification boundary

- Source session: 2026-10-29 through 2026-10-30. Its last response is at `2026-10-30T08:36:31Z`.
- Optional local source: `~/.claude/projects/-Users-fahchen-PersonalProjects-pbeam/d24e9a3b-2db3-4644-a3cd-ae11eec5dc17.jsonl`. The resume command is `claude --resume d24e9a3b-2db3-4644-a3cd-ae11eec5dc17`. This document does not require access to that file.
- Status checked on 2026-10-31 through the GitHub API: issue states, milestone states, project 2 Status fields, and merge commits of the delivery PRs. The four open m25 tasks have no closing PR reference. Native sub-issue membership and milestone membership were also checked.
- Local source checked at `d2d333a9f137efcc1399620dbb6bfbaf9c062214`: lint configuration, test locations, benchmark documentation, and the blaze backlog entry.
- Historical test counts, coverage, timings, and review results below describe their recorded revisions. They are not a new test run on current main. Runtime failures were not reproduced for this documentation change.
