# M24: Redbug CLI handoff

## Goal and scope

Redbug CLI must run on PBeam and trace ERTS nodes. The tool gets a Terra TUI in the controller BEAM, uses the Erlang and Elixir versions of PBeam, and builds one PBeam executable. Terra does not change. Each feature that Terra cannot show is cut or simplified, and the redbug-cli repository records it in `docs/terra-gaps.md`. The GitHub work is [epic #1396](https://github.com/fahchen/pbeam/issues/1396) for the CLI, [epic #1397](https://github.com/fahchen/pbeam/issues/1397) for the run against ERTS nodes, and [epic #1398](https://github.com/fahchen/pbeam/issues/1398) for performance, all under [m24 tracker #1390](https://github.com/fahchen/pbeam/issues/1390). The distribution foundation comes from epics #1391 through #1395.

The code is in the redbug-cli repository ([fahchen/redbug-cli](https://github.com/fahchen/redbug-cli)) on the branch `pbeam`, which starts at `main` `5f7ee1bc`. Each e6 task is one pull request to `pbeam`. The branch merges into `main` only after the final test of the owner, after #1427. [PR #11](https://github.com/fahchen/redbug-cli/pull/11) is the draft of that merge and stays open until the owner approves.

This document gives another agent the goal, the accepted decisions, the verified progress, and the next steps. GitHub issues and the [PBeam project board](https://github.com/users/fahchen/projects/2) remain the source of current scope and status. This document is a dated handoff, not a second task tracker.

## Completion gate

- #1420 through #1426, #1433, and #1446 are accepted after the owner tests the result.
- #1428 runs each step on PBeam and records the first stop of each step. Each gap is a task under #1397.
- The distribution tasks that the run needs are accepted, with tests against real ERTS nodes.
- #1427 builds one PBeam executable with `mix pbeam.build`. A check with a real ERTS peer node passes. The README gives the build and run commands.
- The owner performs the final test of the executable. Then `pbeam` merges into redbug-cli `main`.
- Each pull request passes the redbug-cli local checks and a Fable review with no findings. PBeam pull requests pass `mise run check` with 100% application line coverage.

## Progress summary

| Area | Completed | Remaining |
| --- | --- | --- |
| e6 CLI and TUI | redbug-cli PRs #3 through #10 and #12 are merged into `pbeam`. The Bun TUI and the Phoenix layers are removed. The Terra TUI covers the node screen, sessions, events, console, snippets, presets, and settings. `docs/terra-gaps.md` records 38 cut or simplified features: 9 cut, 28 simplified, 1 restored. | #1427 builds the executable. The owner tests the branch before the merge to `main`. |
| e7 run on PBeam | The checks job is faster (#1465, #1466). The sendfile test no longer hangs (#1464, #1468). | #1428 discovery run, #1467 Linux test hangs, #1480 CI build reuse (PR #1481 open), #1484 benchmark workloads. |
| e8 performance | #1430 compares PBeam with the pinned ERTS (PR #1431). | #1429 reduces a Terra 200x50 frame from the 172 ms baseline to at most 86 ms. It is the last task. |
| Distribution foundation | e1 and e2 are complete. On e3, messages (#1407), `exit/2` (#1458 through PR #1461), links (#1462 through PR #1470), and the link order (#1469 through PR #1471) are merged. | #1459 and #1463 (end the links with `noconnection`), #1409 monitors, #1410 remote spawn and rpc, #1411 nodeup and nodedown, #1412 auto-connect, e4 #1413 through #1417, e5 #1419 ssh tunnel. |

The milestone had 24 closed and 36 open issues on 2026-10-31. The redbug-cli tasks #1420 through #1426, #1433, and #1446 have project Status `In Review` until the owner test. #1427 and #1428 are `Todo`.

## Tasks and current progress

### e6: Redbug CLI with a Terra TUI

Each task is delivered by a merged pull request to the redbug-cli branch `pbeam`. The PBeam task issue stays open until the owner test and the merge to `main`.

| Task | Delivery | Scope |
| --- | --- | --- |
| [#1420](https://github.com/fahchen/pbeam/issues/1420) | redbug-cli #3 | Pins erlang 29.1.1 and elixir 1.20.4-otp-29. |
| [#1421](https://github.com/fahchen/pbeam/issues/1421) | redbug-cli #4 | Loads the injected worker from abstract code with `compile:noenv_forms/2` on OTP 27, 28, and 29 targets. |
| [#1422](https://github.com/fahchen/pbeam/issues/1422) | redbug-cli #5 | `REDBUG_DIST=off` runs the controller without distribution. |
| [#1433](https://github.com/fahchen/pbeam/issues/1433) | redbug-cli #6 | Runs the console on a target without Elixir. |
| [#1423](https://github.com/fahchen/pbeam/issues/1423) | redbug-cli #7 | Starts the Terra TUI in the controller BEAM with the node screen. |
| [#1424](https://github.com/fahchen/pbeam/issues/1424) | redbug-cli #8 | Shows trace sessions and events in the Terra TUI. |
| [#1425](https://github.com/fahchen/pbeam/issues/1425) | redbug-cli #9 | Moves the console, snippets, presets, and settings to the Terra TUI. |
| [#1426](https://github.com/fahchen/pbeam/issues/1426) | redbug-cli #10 | Removes the Bun TUI, the Phoenix and Bandit WebSocket layer, and the musubi stores. |
| [#1446](https://github.com/fahchen/pbeam/issues/1446) | redbug-cli #12 | Keeps the redbug stop message off the TUI screen and shows the stop reason. |
| [#1427](https://github.com/fahchen/pbeam/issues/1427) | None | Builds one `redbug` PBeam executable with `mix pbeam.build`. Waits for #1426, #1428, each gap task, and #1416. |

### e7: Run Redbug CLI against ERTS nodes on PBeam

| Task | State / project Status | Evidence |
| --- | --- | --- |
| [#1428](https://github.com/fahchen/pbeam/issues/1428) | Open / Todo | Discovery run. Builds the release of `pbeam` with distribution on and runs each step: local ERTS node, SSH tunnel, trace with call and return events, event open, console snippet, trace stop. Files one task for each gap. |
| [#1467](https://github.com/fahchen/pbeam/issues/1467) | Open / Todo | Linux CI runs hang at the end: `stops_at_a_part_of_a_channel_that_pbeam_does_not_have` reached a 420 s timeout, and `starts_kernel_and_stops_at_init_stop_as_pinned_erts` hung once in 30 runs. Each test must pass 30 of 30 runs. |
| [#1480](https://github.com/fahchen/pbeam/issues/1480) | PR #1481 open / In Review | Reuses test builds in CI. |
| [#1484](https://github.com/fahchen/pbeam/issues/1484) | Open / Todo | Reduces benchmark workloads in required checks, and keeps the release workloads. |
| [#1436](https://github.com/fahchen/pbeam/issues/1436), [#1464](https://github.com/fahchen/pbeam/issues/1464), [#1465](https://github.com/fahchen/pbeam/issues/1465) | Closed / Done | `process_info/2` current stacktrace item (PR #1445), sendfile test (PR #1468), faster checks (PR #1466). |

### e8: PBeam performance for the Redbug CLI TUI

[#1430](https://github.com/fahchen/pbeam/issues/1430) is Closed / Done through PR #1431. [#1429](https://github.com/fahchen/pbeam/issues/1429) is Open / Todo. It starts with a spike that finds the hot paths. It lands only a change that one interleaved before and after measurement supports. It runs only when no other m24 task is ready and blocks no task.

### Distribution tasks that the run needs

The run of #1428 uses `net_kernel:start/2` with a hidden node, a custom `-epmd_module`, `Node.connect/1`, `rpc:call/4`, `Node.spawn/4`, `net_kernel:monitor_nodes/2`, `:redbug` 2.1.0 on the target, and `ssh:tcpip_tunnel_to_server/6`. The open dependencies are:

| Task | Need |
| --- | --- |
| [#1459](https://github.com/fahchen/pbeam/issues/1459), [#1463](https://github.com/fahchen/pbeam/issues/1463) | End the links of a lost connection with `noconnection`. #1462 is closed through PR #1470. |
| [#1409](https://github.com/fahchen/pbeam/issues/1409) | Monitors across nodes, `net_adm:ping/1`, and `gen_server:call/2` to a remote name. |
| [#1410](https://github.com/fahchen/pbeam/issues/1410) | Remote spawn, `erpc`, and `rpc`. |
| [#1411](https://github.com/fahchen/pbeam/issues/1411) | `nodeup` and `nodedown`, and cleanup after a lost connection. |
| [#1412](https://github.com/fahchen/pbeam/issues/1412) | Connect to a node on the first signal. |
| [#1416](https://github.com/fahchen/pbeam/issues/1416) | Decide how the one-file executable finds an epmd. |
| [#1419](https://github.com/fahchen/pbeam/issues/1419) | OTP `ssh` and the TCP/IP tunnel. |
| [#1413](https://github.com/fahchen/pbeam/issues/1413), [#1414](https://github.com/fahchen/pbeam/issues/1414), [#1415](https://github.com/fahchen/pbeam/issues/1415), [#1417](https://github.com/fahchen/pbeam/issues/1417) | Inbound connections, visible nodes and `global` and `pg`, the atom cache and fragments, and TLS distribution. These tasks are not the minimum for #1428, but the owner asked for more distribution support. |

Row "Distribution" of `docs/erts-differences.md` records the current gaps: no atom cache and fragments (#1415), no `nodeup` and `nodedown` (#1411), no busy output, and entries kept until the run end. A link to a node without a connection gives `noconnection` at once (#1412). The alias owner exit encode error is recorded there.

## Inherited owner decisions

| Decision | Required result | Current result |
| --- | --- | --- |
| Do not modify Terra | Terra stays at commit `30dea5667d7e4ae5777aa3df77acb36c4aac6967`. The CLI adapts to Terra. | All e6 pull requests keep this rule. |
| Cut or simplify unsupported TUI features | Record each feature in redbug-cli `docs/terra-gaps.md` with the original behavior, the reason, the restore work, and the status. | 38 entries on `pbeam`. |
| Deliver one BEAM, not JavaScript | Remove Bun, React, Phoenix, Bandit, and musubi. The TUI runs in the controller BEAM. | Delivered by redbug-cli #10. |
| Keep the final merge behind the owner test | The branch `pbeam` merges into `main` only after #1427 and the owner test. | PR #11 is a draft. Do not merge it. |
| Review a redbug-cli pull request without CI | The main session merges after a Fable review with no findings and the local checks: `mix format --check-formatted`, `mix compile --warnings-as-errors`, and `mix test` in `server/`. | Recorded in epic #1396. redbug-cli has no CI workflow. |
| Treat performance as the lowest priority | Work on the distribution and the CLI before #1429. | #1429 is the last task. |
| Support more distribution than the Redbug minimum | The distribution tasks continue past the minimum of #1428. | e3 and e4 tasks stay open. |
| Keep the e6 task issues open until acceptance | A merged redbug-cli pull request does not close the PBeam task before the owner test. | #1420 through #1426, #1433, and #1446 are `In Review`. |
| Supersede the owner's local redbug-cli edit | The local `mise.toml` edit with OTP 27.2.4 pins is superseded by redbug-cli #4. The owner can drop it. | The uncommitted files `mise.toml`, `TASKS.md`, `PONYTAIL-DEBT.md`, and `docs/screens/` are not part of the branch. Do not touch them. |

## Resume procedure

1. Read `AGENTS.md`, `docs/agents/knowledge.md`, and `docs/agents/patterns.md`. Scan the titles of `docs/agents/improvements.md`. Read `README.md` and `docs/portable-beam-spec.md` before a runtime change.
2. Refresh #1390, #1396, #1397, #1398, and their native children. Check the open PRs #1481 and #1307. Check the redbug-cli branch `pbeam` and PR #11.
3. Continue with the ready PBeam tasks. Start with #1467, then #1484. #1480 waits on PR #1481. Select a new task only when no earlier task is ready.
4. For a distribution task, read the pinned OTP source and run the pinned ERTS probe before the change. Cite the OTP file and line in the pull request. Keep one focused task per pull request.
5. For a redbug-cli change, work on the branch `pbeam`, run the local checks in `server/`, and request one Fable review. Do not merge #11.
6. After the distribution tasks that #1428 needs are accepted, run #1428. File one gap task under #1397 for each stop. Then start #1427.
7. After #1427, ask the owner for the final test. Merge `pbeam` into `main` only after the approval.

Useful commands, from the repository root unless stated:

```sh
mise run issue 1390
mise run issue 1396 Pull\ requests
mise run issue 1397 Tasks
mise run issue 1398 Tasks
gh api repos/fahchen/pbeam/issues/1396/sub_issues
gh pr view 1481 --repo fahchen/pbeam
gh pr view 11 --repo fahchen/redbug-cli
git -C ../redbug-cli log --oneline -10 origin/pbeam
mise run test-focus --test <target>
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
```

For a redbug-cli change, run these commands in `redbug-cli/server`:

```sh
mix format --check-formatted
mix compile --warnings-as-errors
mix test
```

CI runs the full PBeam gate on each pull request. These focused commands are starting points and do not replace it.

## Sources and verification boundary

- Status checked on 2026-10-31 through the GitHub API: issue states, native sub-issue membership, milestone counts, and project Status. The PBeam main branch was at `930743b9`.
- The redbug-cli branch `origin/pbeam` was at `8de5c1b2ebc7bd733f29a87994f2fd1602dcd9e1` (2026-10-30). The branch `pbeam` holds redbug-cli pull requests #3 through #12, all merged into `pbeam`.
- [PR #11](https://github.com/fahchen/redbug-cli/pull/11) records the verification of 2026-10-30: `mix compile --warnings-as-errors` and `mix test` pass with 115 tests and a live target node. PTY runs of the release on ERTS 29.1.1 exit 0. On PBeam with `REDBUG_DIST=off`, each step passes, and quitting exits 0.
- No test, benchmark, or PTY run was repeated for this document. The values above describe their recorded revisions.
- GitHub issues and the project board stay current. Update this document when the goal, a decision, an acceptance criterion, or the next step changes.
