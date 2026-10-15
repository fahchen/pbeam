# Performance status

This file is the one status document of the performance programme. It holds one entry for each landed performance change of the interpreter dispatch plan (`docs/interpreter-dispatch-plan.md`) and of the term performance proposal (`docs/term-performance-design-codex-v2.md`), with the before value and the after value of the affected shapes. The detailed measurement record is `docs/interpreter-dispatch-measurements.md`. The task status stays in GitHub Issues.

## The rule of this file

- The newest entry is first. Order a new entry by the date of the merge that lands the change.
- One entry for each landed performance change. A change that only edits documentation takes no entry, because it has no measured effect. An entry for a change that is not landed carries the marker **Not landed** and states the reason.
- Every number names its revision, its session or run group, and its load. A run group is one measurement session with one host load. Two numbers from two sessions are not comparable, because the run-to-run noise is about 10 percent at 200000 iterations.
- A column boundary in a cumulative table is not a measured gain of the later step. Two neighbouring columns come from different run groups, so their difference holds both the change of the code and the noise of the two runs.
- A claim that a change moved a shape needs one session that interleaves the two revisions. One session of one revision alone cannot support that claim.
- Every number comes from a source in this repository or from a merged pull request, and the entry names that source. This file invents no number and merges no two sessions into one comparison.

The host of every number is Darwin 27.0.0 arm64, an Apple M1 Max with 10 CPUs, unless an entry names another host. The pinned toolchain is `rustc 1.99.0` and `cargo 1.99.0`, and the pinned ERTS is OTP 29, ERTS 17.1, `jit`, `schedulers 10`, `logical processors 10`. The environment table is in `docs/interpreter-dispatch-measurements.md`.

## Current state

The table holds the headline numbers of the whole programme. Each row names its revision, its session or run group, and its load or run rule. A row with two numbers from two sessions states both sessions, because the two numbers are not comparable.

| Headline number | Value | Revision | Session or run group | Load or run rule | Source |
| --- | --- | --- | --- | --- | --- |
| External `call_ext_only` floor loop, 200000 iterations | 122.5 ns, from 257.0 ns | `b8c71940` before, `8e1b44b4` after | One interleaved session of eight rounds, each number the minimum of nine probe runs inside one invocation | One-minute load 2.03 to 2.48 | Issue [#628](https://github.com/fahchen/pbeam/issues/628) and the record |
| CPU time for each release Terra key press, 60 keys | 24.188 ms, from 25.896 ms | `b8c71940` before, `8e1b44b4` after | A second interleaved session of eight rounds, the mean of the raw CPU time with no idle correction | One-minute load 1.09 to 1.97 | Issue [#628](https://github.com/fahchen/pbeam/issues/628) and the record |
| `call_ext_only` floor loop, 200000 iterations | 149.3 ns, from 257.1 ns | `b8c71940` before, after on the branch `perf/627-process-slot` (`ef77e031`) | One interleaved session of eight rounds and two builds, each number the minimum of nine probe runs inside one invocation and the minimum of the eight rounds | One-minute load 2.40 to 2.97 | Issue [#627](https://github.com/fahchen/pbeam/issues/627) and the record |
| Local `call_only` floor loop, 200000 iterations, task #627 | 42.2 ns, from 102.5 ns | `b8c71940` before, after on the branch `perf/627-process-slot` (`ef77e031`) | The same interleaved session and the same rule | The same loads | Issue [#627](https://github.com/fahchen/pbeam/issues/627) and the record |
| Position `call_only` floor loop, 200000 iterations, task #627 | 42.1 ns, from 102.5 ns | `b8c71940` before, after on the branch `perf/627-process-slot` (`ef77e031`) | The same interleaved session and the same rule | The same loads | Issue [#627](https://github.com/fahchen/pbeam/issues/627) and the record |
| CPU time for each release Terra key press, 60 keys | 18.00 ms, from 26.22 ms (mean of three runs) | `b8c71940` before, after on the branch `perf/627-process-slot` (`ef77e031`) | A second interleaved session (s2) with a check of the other host processes, three accepted rounds of 60 keys for each build, raw CPU time of the key window from `ps` | One-minute load 1.11 to 1.44 before each run | Issue [#627](https://github.com/fahchen/pbeam/issues/627) and the record |
| The `select_val` probe with 256 choices, 200000 iterations | 526.2 ns, from 984.2 ns | `915590af` before, after on the branch `perf/545-select-val`, landed as `8659710f` | One interleaved session of three rounds and three builds, each number the minimum of nine probe runs inside one invocation | One-minute load 3.28 to 7.20 | Issue [#545](https://github.com/fahchen/pbeam/issues/545) and the record |
| The `select_val` probe with 16 choices, 200000 iterations | 500.2 ns; the bound of 32 keeps the linear form, so the value is the linear row of the session | `915590af` before, after on the branch `perf/545-select-val`, landed as `8659710f` | The same interleaved session and the same rule; the ordered rows of the session measure the bound of 10 and do not apply to the landed change | The same loads | Issue [#545](https://github.com/fahchen/pbeam/issues/545) and the record |
| The `select_val` linear bound, `SELECT_VALUE_LINEAR_LIMIT` | 32 | The branch `perf/545-select-val`, landed as `8659710f` | One estimate between the two end-to-end points of the interleaved session and the micro-measurement point at 32 choices | One-minute load 3.28 to 7.20 | Issue [#545](https://github.com/fahchen/pbeam/issues/545) and the record |
| Local `call_only` floor loop of the #600 check session, 200000 iterations; an older session, and the task #627 row replaces it as the current value | 117.9 ns | `ddb32f6e` | The interleaved check session, the minimum of three rounds, and each round is the minimum of nine probe runs inside one invocation | One-minute load 26.15, 29.92, and 25.70 on the three rounds | [#600](https://github.com/fahchen/pbeam/pull/600) and `docs/interpreter-dispatch-measurements.md` |
| Position `call_only` floor loop of the #600 check session, 200000 iterations; an older session, and the task #627 row replaces it as the current value | 117.8 ns | `ddb32f6e` | The same interleaved check session | The same three rounds | [#600](https://github.com/fahchen/pbeam/pull/600) and the record |
| Local `call_only` floor loop of the after-revision of #585, 200000 iterations | 118.4 ns | `d54e6a4` | The same interleaved check session | The same three rounds | [#600](https://github.com/fahchen/pbeam/pull/600) and the record |
| Position `call_only` floor loop of the after-revision of #585, 200000 iterations | 118.4 ns | `d54e6a4` | The same interleaved check session | The same three rounds | [#600](https://github.com/fahchen/pbeam/pull/600) and the record |
| Row 9 gate of the term plan, at most 120 ns for each iteration | Pass at the minimum across the three rounds | `d54e6a4` and `ddb32f6e` | The same interleaved check session | Every value of rounds 2 and 3 is below 120 ns; round 1 is above the gate for `d54e6a4` only | [#600](https://github.com/fahchen/pbeam/pull/600) and the record |
| Step 0 position `call_only` floor loop, 200000 iterations | 506.8 ns | `078c119` | The step 0 run, the minimum of nine probe runs of one probe | One benchmark run at load `{ 5.05 6.37 5.81 }` | [#520](https://github.com/fahchen/pbeam/pull/520) and the record |
| Step 0 local `call_only` floor loop, 200000 iterations | 337.5 ns | `078c119` | The same step 0 run | The same run | [#520](https://github.com/fahchen/pbeam/pull/520) and the record |
| Pinned ERTS `call_only` floor loop, 200000 iterations | 2.46 ns to 2.58 ns across the recent sessions | The after session of [#569](https://github.com/fahchen/pbeam/pull/569), [#576](https://github.com/fahchen/pbeam/pull/576), [#585](https://github.com/fahchen/pbeam/pull/585), and [#598](https://github.com/fahchen/pbeam/pull/598) | The ERTS run of the same session as each PBeam number | The load of that session | The tables of those pull requests and the record |
| Terra first frame | 0.52 s | `e74a1af9` | One run of `bash scripts/terra_latency.sh release` | Load `{ 12.19 25.63 26.34 }`, above the bound of 12, so the record marks the run as weak | [#588](https://github.com/fahchen/pbeam/pull/588) and the record |
| Terra key latency, four keys | 0.014 s to 0.033 s | `e74a1af9` | The same single run | The same load | [#588](https://github.com/fahchen/pbeam/pull/588) and the record |
| Peak live bytes of one Terra scene | 24096 bytes; 20192 bytes after the compact term landed as `855da1fc` on 2026-10-14 | 24096 at `ffb1a05a` and at `35c410e6`; 20192 at `dabac942` | Two runs of `mise run bench-live-bytes` at `ffb1a05a`, spread 0, and 30 runs of each revision at `35c410e6` and `dabac942`, spread 0 | Load `{ 41.79 77.93 80.63 }` and `{ 38.77 76.71 80.18 }` at `ffb1a05a`; load 6.13 to 6.76 at the two later revisions | [#589](https://github.com/fahchen/pbeam/pull/589) and [#598](https://github.com/fahchen/pbeam/pull/598) |
| `size_of::<Term>()` | 40 bytes; 32 bytes after the compact term landed as `855da1fc` on 2026-10-14 | 40 at `35c410e6`, and 40 bytes before and after the host-term boundary of #582; 32 at `dabac942` | `RUSTC_BOOTSTRAP=1 cargo rustc --locked --lib -- -Zprint-type-sizes` at the pinned rustc | No load applies to a type size | [#582](https://github.com/fahchen/pbeam/pull/582), [#598](https://github.com/fahchen/pbeam/pull/598), and `docs/term-layout-design.md` |
| Maximum scheduler step pause on the exit of a process, list of 1000000 cells | 56.541 µs, from 23664.417 µs | Before `3b3a21d`, after `f899ea91` | `mise run bench-release`, the maximum of five runs of one path | Before load `{ 4.95 45.12 66.44 }` to `{ 4.64 44.39 66.06 }`; after load `{ 6.23 51.10 69.48 }` to `{ 5.45 48.73 68.31 }` | [#578](https://github.com/fahchen/pbeam/pull/578) |
| ETS `set` lookup of one object in a table of 1000 objects | 10.000 ns, from 4195.205 ns | Before `3b3a21d`, after `0c27d4d` | Three runs of the micro-measurement, the minimum of the nine timed runs of one run | Load before `{ 4.64 6.07 6.76 }`, after `{ 11.75 22.48 17.40 }` | [#570](https://github.com/fahchen/pbeam/pull/570) |
| Row 6 gate of the term plan | Fail on its numbers: the median of the first frame is 0.37 s in both revisions, and the peak-live-byte reduction is 16.20 percent, below the 20 percent bound; the owner decided to land the compact term as the first half of the 16-byte target | Before `35c410e6`, after `dabac942` | 30 runs of the live-byte harness and 30 runs of the Terra path of each revision, with one before run and one after run next to each other in each round | Live-byte load 6.13 to 6.76; Terra load of the 60 runs 3.23 to 31.34, and 15 runs above the bound of 12; floor-loop load `{ 6.45 6.73 4.81 }` | [#598](https://github.com/fahchen/pbeam/pull/598) |

The step 0 numbers and the `ddb32f6e` numbers come from two different sessions. Their difference is not a measured gain of one step, and this file does not state one.

## Entries

Each entry is one landed change or one change that is not landed. The entries are newest first.

### 2026-10-15 18:45 JST - Not landed: take the source term of a move whose source is dead, task [#631](https://github.com/fahchen/pbeam/issues/631), branch `perf/631-move-take`

The change marks a `move` whose source register no later instruction reads at load time, and the interpreter takes the term out of the source with no reference count change. On a Terra key press, 27,472 of the 46,835 moves of a boxed term have a dead source. The change is **Not landed**: its target, the CPU time of a Terra key press, shows no gain beyond the session noise, and the floor shapes fall by 0.4 to 3.6 percent only. Commit `c58b8789` reverts commit `4fc08860` on the branch, and `docs/backlog.md` keeps the change with a trigger.

One interleaved session on 2026-10-15 from 18:21:44 to 18:25:28 JST, before `e93523c3` and after `4fc08860`. The floor rows are the minimum of six rounds in rotated order, with a one-minute load of 3.74 to 4.15. The Terra row is rounds 2 to 8 of a second interleaved session of 60 keys from 18:25:34 to 18:43:07 JST, with a one-minute load of 2.49 to 7.34 and the raw CPU time of `pbeam`. The session checked for `cargo`, `rustc`, and `nextest` processes only before each run, and a build of other work probably overlapped round 1 of `4fc08860`, so the entry leaves out the round 1 pair. The source is the section "The source term of a move whose source is dead, task #631" of `docs/interpreter-dispatch-measurements.md`.

| Shape | Before `e93523c3` | After `4fc08860` | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, 200000 iterations | 47.7 ns | 47.5 ns | -0.4 percent |
| Local `call_only` floor loop, 200000 iterations | 42.3 ns | 41.8 ns | -1.2 percent |
| Position `call_only` floor loop, 200000 iterations | 42.3 ns | 41.7 ns | -1.4 percent |
| `call_only` plus the `bif` of `map_get/2` on a 1000-key map | 105.4 ns | 101.6 ns | -3.6 percent |
| CPU time for each release Terra key press, rounds 2 to 8 | 14.857 ms | 15.048 ms | +1.3 percent at the mean, -1.1 percent at the median |

The Terra rounds of one binary span 11.667 ms to 16.500 ms. The `## Current state` table does not change.

### 2026-10-15 16:35 JST - Fetch the map elements in one walk with no allocation, task [#629](https://github.com/fahchen/pbeam/issues/629), pull request [#639](https://github.com/fahchen/pbeam/pull/639)

The loader sorts two or more pairs of `get_map_elements` in flatmap key order, as the ERTS loader does. The arm keeps one copy of the map handle, walks a flatmap once, and writes each value when it finds the key, with no intermediate vector. `map_hash` allocates nothing for an atom, `[]`, a small integer, or a binary key.

One interleaved session on 2026-10-15 from 16:19:34 to 16:24:45 JST, before `e93523c3` and after `028221a9`. The floor rows are the minimum of six rounds in rotated order, and each round is the minimum of nine probe runs inside one invocation; the one-minute load is 2.41 to 2.53. The map rows come from a scratch module with 3000000 iterations of one `get_map_elements` of three atom keys, the minimum of six rounds of user CPU time; the one-minute load is 2.30 to 2.64. The Terra row comes from a second interleaved session on 2026-10-15 from 16:25:04 to 16:34:15 JST with the same two binaries. It is the mean of eight rounds of 60 keys in rotated order, with the raw CPU time of `pbeam` and no idle correction; the one-minute load is 1.06 to 2.49. The source is the section "The map elements in one walk, task #629" of `docs/interpreter-dispatch-measurements.md`.

| Shape | Before `e93523c3` | After `028221a9` | Change |
| --- | --- | --- | --- |
| Three keys of a 6-key flatmap, 3000000 iterations | 0.75 s | 0.53 s | -29.3 percent |
| Three keys of a 43-key hashmap, 3000000 iterations | 0.92 s | 0.59 s | -35.9 percent |
| `call_only` plus `map_get/2` on a 1000-key map, 200000 iterations | 102.4 ns | 88.1 ns | -14.0 percent |
| `call_ext_only` floor loop, 200000 iterations | 47.7 ns | 47.6 ns | -0.2 percent |
| Local `call_only` floor loop, 200000 iterations | 42.4 ns | 42.1 ns | -0.7 percent |
| CPU time for each release Terra key press | 15.896 ms | 15.042 ms | -5.4 percent, not significant (Welch t = 1.68, p = 0.12 over 8 rounds) |

The ranges of the six rounds do not overlap for the three map rows. No other floor shape becomes slower. The Terra ranges of the eight rounds overlap (14.333 to 16.500 ms before, 12.667 to 16.333 ms after), and the after round is below the before round in six of the eight rounds, so this session does not resolve the Terra change: a Welch t test gives t = 1.68 and p = 0.12, and a paired t test of the rounds gives t = 1.54 and p = 0.17. The first session of four rounds gave 15.792 ms and 15.125 ms (-4.2 percent).

### 2026-10-15 17:30 JST - Reuse the Y register storage of a released frame, task [#630](https://github.com/fahchen/pbeam/issues/630), pull request [#640](https://github.com/fahchen/pbeam/pull/640) - Not landed

The change keeps up to 8 empty slot arrays of at most 32 slots for each process, from the frames that a return or a tail call releases, and the `allocate` of a frame with no slot array takes one of them. It does not land: the CPU time for each release Terra key press, which is the shape that the change targets, shows no fall beyond the noise of the session, and the floor loops, which have no `allocate`, are slower. A body-call probe with two `allocate` instructions in each iteration is 22 percent faster.

One interleaved session on 2026-10-15 from 17:20:30 to 17:27:45 JST, before `e93523c3` and after `1c344478` (reverted by `00e3f44e`). The floor rows are the minimum of eight rounds in rotated order, and each round is the minimum of nine probe runs inside one invocation. The Terra row is the mean of six rounds of 60 keys in rotated order, with the raw CPU time of `pbeam` and no idle correction. The one-minute load is 1.02 to 2.93. The body-call row comes from a second interleaved session on 2026-10-15 from 17:30:55 to 17:31:07 JST, with the same two binaries, eight rounds in rotated order, and a one-minute load of 3.50 to 3.83. The source is the section "The Y register storage of a released frame, task #630" of `docs/interpreter-dispatch-measurements.md`.

| Shape | Before `e93523c3` | After `1c344478` | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, 200000 iterations | 47.7 ns | 49.1 ns | +2.9 percent |
| Local `call_only` floor loop, 200000 iterations | 42.4 ns | 44.2 ns | +4.2 percent |
| Position `call_only` floor loop, 200000 iterations | 42.4 ns | 44.2 ns | +4.2 percent |
| CPU time for each release Terra key press | 15.667 ms | 15.472 ms | -1.2 percent, ranges overlap |
| Body-call probe, user CPU time of 3000000 iterations, median of 8 rounds | 0.76 s | 0.59 s | -22 percent |

### 2026-10-15 13:00 JST - Call an import through a resolved export target, task [#628](https://github.com/fahchen/pbeam/issues/628), pull request [#636](https://github.com/fahchen/pbeam/pull/636)

The loader keeps the imports of a module with the call that it decides for each import, and `Instruction::CallExternal` holds the import index. The code table of a run keeps the export target of each import of each loaded module after the first call that finds it, so a later `call_ext` of an export in module code makes no lookup by name and compares no name. A call of a PBeam BIF, which has no `BifId`, still matches its name. A call that finds no export keeps the slot empty, so a module that loads later is found by the next call.

One interleaved session on 2026-10-15 from 12:51:44 to 12:55:58 JST, before `b8c71940` and after `8e1b44b4`. The floor rows are the minimum of eight rounds in rotated order, and each round is the minimum of nine probe runs inside one invocation; the one-minute load is 2.03 to 2.48. The Terra row comes from a second interleaved session on 2026-10-15 from 13:06:37 to 13:15:49 JST with the same two binaries. It is the mean of eight rounds of 60 keys in rotated order, with the raw CPU time of `pbeam` and no idle correction; the one-minute load is 1.09 to 1.97. The source is the section "The resolved export target of an import, task #628" of `docs/interpreter-dispatch-measurements.md`.

| Shape | Before `b8c71940` | After `8e1b44b4` | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, 200000 iterations | 257.0 ns | 122.5 ns | -52.3 percent |
| Local `call_only` floor loop, 200000 iterations | 102.0 ns | 102.1 ns | +0.1 percent |
| Position `call_only` floor loop, 200000 iterations | 101.8 ns | 102.0 ns | +0.2 percent |
| CPU time for each release Terra key press | 25.896 ms | 24.188 ms | -6.6 percent |

The medians of the two `call_only` rows move by +0.7 percent. Of the added BIF shapes, `hd/1` moves by +1.1 percent and `tuple_size/1` by -2.0 percent at the minimum, and the ranges of the eight rounds of these two shapes do not overlap. The `hd/1` shape is thus a measured regression, probably from code layout, because the change does not touch the `bif` path. The Terra ranges of the eight rounds overlap (22.667 to 27.333 ms before, 21.500 to 25.167 ms after), and the after round is below the before round in six of the eight rounds, so the Terra number is weaker than the floor number. The first session of three rounds gave 26.833 ms and 24.278 ms with no idle correction (-9.5 percent); its reported 23.718 ms came from an idle correction of one 10 ms tick of `ps`, which the record now rejects.

### 2026-10-15 12:20 JST - Get the current process with no hash, task [#627](https://github.com/fahchen/pbeam/issues/627), branch `perf/627-process-slot`

The change keeps each live process in a slot of the process table, with an index from the pid to the slot, and keeps the slot of the current process in the table. A read of the current process then makes no hash, as the pinned ERTS reads the current process through a pointer (`erl_process.h:698`, OTP 29.1.1). Only a lookup of a process by its pid hashes the pid. The measured before revision is `b8c71940`, the head of `origin/main` that the branch starts from, and the after revision is `ef77e031`, the commit of the change.

| Shape | Before `b8c71940` | After `ef77e031` | Change |
| --- | --- | --- | --- |
| `call_ext_only` floor loop, ns for each of 200000 iterations | 257.1 | 149.3 | −107.8 ns, −41.9 percent |
| Local `call_only` floor loop, ns for each of 200000 iterations | 102.5 | 42.2 | −60.3 ns, −58.8 percent |
| Position `call_only` floor loop, ns for each of 200000 iterations | 102.5 | 42.1 | −60.4 ns, −58.9 percent |
| CPU time for each release Terra key press, mean of three runs of 60 keys | 26.22 ms | 18.00 ms | −8.22 ms, −31.4 percent |

Two sessions interleave the two release builds on 2026-10-15, with a rotated order in each round and no other `cargo`, `rustc`, or `nextest` process. Session s1 (11:55:04 to 11:55:41 JST) gives the floor numbers: eight rounds at a one-minute load of 2.40 to 2.97, and each floor number is the minimum of nine probe runs inside one invocation and the minimum of the eight rounds. Session s2 (12:08:34 to 12:13:10 JST) gives the key numbers: three accepted rounds at a one-minute load of 1.11 to 1.44 before each run, with no other `erl`, `beam`, or `python` process and no burst of new host processes in a run. Session s2 replaces the key rounds of s1, because round 3 of s1 ran while the host started about 20000 processes. The ranges of the two revisions do not overlap for any floor shape or added row, and every after key run (17.33 to 18.33 ms) is below every before key run (26.00 to 26.33 ms).

Source: the section "The current process with no hash, task [#627](https://github.com/fahchen/pbeam/issues/627)" of `docs/interpreter-dispatch-measurements.md`. Known limits: the CPU time comes from `ps` with a resolution of 10 ms, or about 0.17 ms for each key over 60 keys; the session does not measure a workload with many processes.

### 2026-10-15 01:10 JST - The `select_val` dispatch decision, task [#545](https://github.com/fahchen/pbeam/issues/545), pull request [#622](https://github.com/fahchen/pbeam/pull/622) - Landed as `8659710f`

The change adds a prepared form of the `select_val` choice list and removes the work-vector allocation of the term comparison that the ordered form uses. The decoder builds the prepared form one time at load time. A list of at most `SELECT_VALUE_LINEAR_LIMIT` choices keeps the encoded order and the linear walk of every earlier revision. A larger list is sorted by the ERTS term order and the interpreter binary-searches it. A duplicate choice keeps the linear form, so the first-match result of ERTS is preserved. The measured before revision is `915590af`, the head of `origin/main` that the branch starts from.

`SELECT_VALUE_LINEAR_LIMIT` is 32. The bound is a measured estimate between the two end-to-end points of the task: the linear form wins at 16 choices and the ordered form wins by 458.005 ns at 256 choices, and the only micro-measurement point between them is 32 choices, where the linear walk costs 152.053 ns in the worst case and the ordered search costs 34.172 ns. The pinned ERTS uses 10 (`erts/emulator/beam/emu/generators.tab:240-254`, OTP 29.1.1), because its ordered search compares one encoded value at a time and costs less than the `Term::compare` call of PBeam. No end-to-end probe covers 17 to 32 choices, so a later session can pin the bound with end-to-end probes at 32 and 64 choices.

The shape distribution comes from `escript /tmp/select_val_histogram.escript fixtures <OTP lib dir> <Elixir lib dir>` on the pinned ERTS: 1777 BEAM files and 24568 `select_val` instructions, of which 82.82 percent hold 1 to 4 choices, 8.36 percent hold 5 to 8, 5.16 percent hold 9 to 16, 2.15 percent hold 17 to 32, 0.77 percent hold 33 to 64, and 0.75 percent hold 65 or more. Only 373 instructions, or 1.52 percent, hold more than 32 choices.

The micro-measurement is `select_val_scan_*` and `select_val_binary_search_*` of `tests/runtime/interpreter/data_structure_costs.rs`, inside the test `prints_the_cost_of_every_derivation`. Each invocation is the minimum of nine timed runs of 100000 operations. The linear column and the after column come from one release invocation at `d8ffdb36`, after the compare fast path, and the record of that invocation holds no load. The before column is the minimum of three release invocations in the first session of the task, which measured the code of `b4b1d275` at a one-minute load of 4.39 to 6.04. The value is the last choice, which is the worst case of the linear walk and the `Greater` case of the binary search.

| Choices | Linear walk, ns for one operation | Ordered binary search before, ns for one operation | Ordered binary search after, ns for one operation |
| --- | --- | --- | --- |
| 4 | 19.828 | 48.4 | 16.381 |
| 16 | 78.440 | 116.1 | 27.791 |
| 32 | 152.053 | 139.6 | 34.172 |
| 64 | 292.362 | 161.8 | 38.867 |
| 128 | 576.458 | 186.0 | 44.432 |
| 256 | 1145.774 | 207.2 | 50.515 |

A branch session re-measures the same test at a one-minute load of 2.10 to 2.88 and measures the ordered rows at 9.371, 16.341, 19.827, 23.353, 26.825, and 30.284 ns, below the after column, so the after column is conservative.

The end-to-end measurement is `tests/integration/select_val.rs` on the fixture `fixtures/dispatch/select_val_probe.erl`. `ordered_loop/2` runs a `select_val` with 256 choices one time for each iteration, `nibble_loop/2` runs one with 16 choices, and `linear_loop/2` is the control with a 2-choice `select_val`. The session interleaves three builds in one worktree, and the `src/` of each build is checked out into the one worktree: the before build is `915590af`, which keeps the linear walk for every shape; the middle build is `b4b1d275`, the ordered form without the compare fast path; the after build is `d8ffdb36`, the ordered form with the fast path. Each number is the minimum of nine probe runs inside one invocation of the release test, and the number divides the run by the 200000 iterations.

| Round | Before `915590af`, 256 | Middle `b4b1d275`, 256 | After `d8ffdb36`, 256 | Before, 16 | Middle, 16 | After, 16 | Before, 2 | Middle, 2 | After, 2 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 992.740 | 672.733 | 526.883 | 506.625 | 584.573 | 517.379 | 488.130 | 489.851 | 487.776 |
| 2 | 990.376 | 672.988 | 528.557 | 507.679 | 582.008 | 518.964 | 489.019 | 491.837 | 491.222 |
| 3 | 984.243 | 684.754 | 526.238 | 500.169 | 583.174 | 514.915 | 486.306 | 488.149 | 489.312 |
| Minimum | 984.243 | 672.733 | 526.238 | 500.169 | 582.008 | 514.915 | 486.306 | 488.149 | 487.776 |

The one-minute load of the three builds is `{ 4.40 5.95 7.00 }`, `{ 3.71 5.72 6.90 }`, and `{ 3.45 5.60 6.84 }` in round 1; `{ 3.28 5.46 6.77 }`, `{ 7.20 6.22 7.02 }`, and `{ 6.16 6.03 6.94 }` in round 2; and `{ 5.53 5.90 6.88 }`, `{ 5.93 5.96 6.88 }`, and `{ 5.17 5.80 6.81 }` in round 3.

The 256-choice shape is direct for the landed change: it is above both bounds, so it keeps the ordered form in the middle and the after build, and it falls 458.005 ns, or 46.5 percent, against the before build. The 16-choice shape keeps the linear form under the landed bound, so its landed value is the linear arm of the before build, 500.169 ns; that arm is the code of this branch, and the run of the final build does not measure 16 choices, so the value is an inference from the same session and not a measurement of the final revision. The ordered rows of the session measure the bound of 10 and do not apply to the landed change. The control moves by at most 1.843 ns, or 0.4 percent, across the three builds.

Source: the section "The `select_val` dispatch decision of task [#545](https://github.com/fahchen/pbeam/issues/545)" of `docs/interpreter-dispatch-measurements.md` and the task branch. Known limits: the bound of 32 is an estimate, because no end-to-end probe covers 17 to 32 choices; `docs/backlog.md` records that gap with its trigger. The fixture measures the 256-choice shape, the 16-choice shape, and the 2-choice control, so no number here covers a `select_val` with 6878 choices.

### 2026-10-14 23:50 JST - Shared append tree, task [#558](https://github.com/fahchen/pbeam/issues/558), pull request [#620](https://github.com/fahchen/pbeam/pull/620) - Landed as `a4072f1f`

One interleaved session of `tests/integration/binary_append_bench.rs` on the release profile, three rounds, and each number is the minimum of nine timed runs inside one invocation. The before revision is `915590af` with the benchmark files of the branch, and the after revision is the branch `perf/558-append-tree` at `eb1dd92a`. The one-minute load of the rounds is 7.96 to 9.36.

| Shape | Before `915590af` | After `eb1dd92a` | Change |
| --- | --- | --- | --- |
| Append to a 100000-byte prefix, one byte each, ns for each append | 2567.920 | 948.750 | -63.1 percent |
| Append to a 1000000-byte prefix, one byte each, ns for each append | 19854.150 | 1672.950 | -91.6 percent |
| Doubling build of 1000 bytes, total ns | 26125 | 25250 | -3.3 percent |
| Doubling build of 100000 bytes, total ns | 40292 | 34625 | -14.1 percent |
| Doubling build of 1000000 bytes, total ns | 99292 | 60417 | -39.2 percent |
| Warm match of a 1000000-byte subject, 1 round, total ns | 336959 | 331959 | -1.5 percent |
| Warm match of a 1000000-byte subject, 100 rounds, total ns | 32223292 | 32061166 | -0.5 percent |
| First match of a fresh 100000-byte subject, total ns | 50125 | 51875 | +3.5 percent |
| First match of a fresh 1000000-byte subject, total ns | 337958 | 354542 | +4.9 percent |

The view descriptor grows from 32 bytes to 56 bytes, because the view now holds the chunk list and the joined array; a view that no append built keeps both fields empty or unset, and the test `prints_the_cost_of_a_binary_view` records the larger descriptor. The gate of the task is that a shared append copies no old payload byte except one partial 4096-byte leaf, and that the match scan throughput regresses by at most 10 percent. The copy and the three build shapes fall, and each match shape is inside the 10 percent bound.

### 2026-10-14 23:40 JST - Store only the atom index in the atom term, task [#596](https://github.com/fahchen/pbeam/issues/596), pull request [#619](https://github.com/fahchen/pbeam/pull/619) - Landed as `e0d9cbe5`

This change reaches the 16-byte term that the owner set as the acceptance of row 6 of the term plan. The pinned toolchain is rustc 1.99.0, the host is Darwin 27.0.1 arm64, Apple M1 Max, and the before revision is `6fd1b3db`.

The type sizes come from `RUSTC_BOOTSTRAP=1 mise exec -- cargo rustc --locked --lib -- -Zprint-type-sizes`.

| Type | Before `6fd1b3db` | After `f2cadf82` |
| --- | --- | --- |
| `term::Term` | 32 bytes | 16 bytes |
| `Option<term::Term>` | 32 bytes | 16 bytes |
| `term::atom::Atom` | 24 bytes | 4 bytes |

The peak live bytes of one Terra scene come from `mise run bench-live-bytes` on each revision, at load `{ 2.58 4.44 5.55 }`, with a spread of 0 over the two runs of each revision.

| Revision | Peak live bytes | Peak resident set size |
| --- | --- | --- |
| `6fd1b3db` | 20192 | 3952 KB |
| `f2cadf82` | 11904 | 3792 KB |

The dispatch numbers come from one interleaved session of the `dispatch_baseline` probes, three rounds, and each number is the minimum of nine probe runs inside one invocation. The one-minute load of every round is 3.35 to 4.08.

| Shape, 200000 iterations | Before `6fd1b3db` | After `f2cadf82` |
| --- | --- | --- |
| Local `call_only` floor loop | 100.4 ns, 99.6 ns, 100.3 ns | 112.3 ns, 112.4 ns, 112.4 ns |
| Position `call_only` floor loop after 20 guest functions | 100.4 ns, 100.1 ns, 101.5 ns | 112.4 ns, 112.6 ns, 112.3 ns |
| Added `call_only` floor loop plus `gc_bif '+'` | 51.6 ns, 53.1 ns, 53.1 ns | 66.7 ns, 66.6 ns, 66.8 ns |

The local shape rises 12.0 percent and the position shape rises 12.1 percent. The local shape stays below the 120 ns gate of the term plan and it is below the 117.9 ns value that the headline table holds for `ddb32f6e`. Those two numbers come from two sessions, so their difference is not a measured change of this step.

The Terra Counter latency comes from one interleaved session of `bash scripts/terra_latency.sh release`, 15 rounds per revision, at a one-minute load of at most 3.4. The first frame is 0.34 s at the median in both revisions, and a key answers in 27.0 ms before and 26.0 ms after. The spread of the two groups overlaps, so this session measures no change of the Terra path. The issue asks for at least 30 rounds of each revision, and this session holds 15; the entry states the smaller count.

### 2026-10-14 22:32 JST - Borrowed-argument leaf lane, task [#568](https://github.com/fahchen/pbeam/issues/568), pull request [#616](https://github.com/fahchen/pbeam/pull/616) - Landed as `ef0c8b94`

One interleaved session of the `dispatch_baseline` probes. Each round runs the before binary and then the after binary. Every number is the minimum of nine probe runs inside one invocation. The before revision is `6935019a`, and the after revision is `6ef206e8` on the branch `perf/568-leaf-lane`. The one-minute load of the rounds is 5.29 to 8.21.

| Shape, 200000 iterations | Before `6935019a` | After `6ef206e8` | Change |
| --- | --- | --- | --- |
| Local `call_only` floor loop | 123.2 ns, 124.0 ns, 122.1 ns | 105.1 ns, 104.2 ns, 106.1 ns | -14.6 percent at the minimum, -15.0 percent at the median |
| Position `call_only` floor loop after 20 guest functions | 123.3 ns, 121.7 ns, 122.6 ns | 104.5 ns, 104.5 ns, 109.1 ns | -14.2 percent at the minimum, -14.7 percent at the median |
| Added `call_only` floor loop plus `gc_bif '+'` | 71.6 ns, 68.1 ns, 72.1 ns | 55.9 ns, 55.6 ns, 57.0 ns | -18.4 percent at the minimum |

The gate of the task is that the local `call_only` floor loop falls against the base group of the same session, and that the added delta of `gc_bif '+'` stays below 80 ns. Both hold in each of the three rounds.

### 2026-10-14 - Row 6 compact term, task [#571](https://github.com/fahchen/pbeam/issues/571), pull request [#598](https://github.com/fahchen/pbeam/pull/598) - Landed as `855da1fc`

The change boxes the 32-byte binary descriptor behind one `Arc` to a private `BinaryView`, so the `Term::Binary` arm falls from 32 bytes to 8 bytes. The owner decided to land the compact term as the first half of the 16-byte target of row 6, and the row gate fails on its numbers. Pull request [#598](https://github.com/fahchen/pbeam/pull/598) merged on 2026-10-14 as the squash commit `855da1fc` of a branch whose measured revisions are `35c410e6` (before) and `dabac942` (after). The 16-byte target waits for the immediate atom root of [#580](https://github.com/fahchen/pbeam/issues/580).

| Shape | Before | After | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| `size_of::<Term>()` | 40 bytes | 32 bytes | -8 bytes | Before `35c410e6`, after `dabac942` | Pinned rustc 1.99.0, `-Zprint-type-sizes`; the type size carries no load |
| `size_of::<Option<Term>>()` | 40 bytes | 32 bytes | -8 bytes | The same two revisions | The same command |
| `size_of::<Binary>()` | 32 bytes | 8 bytes | -24 bytes | The same two revisions | The same command |
| `size_of::<BinaryView>()` | No separate type | 32 bytes | A new type | The same two revisions | The same command |
| `size_of::<MatchState>()` | 40 bytes | 16 bytes | -24 bytes | The same two revisions | The same command |
| `size_of::<Operand>()` | 40 bytes | 32 bytes | -8 bytes | The same two revisions | The same command |
| `size_of::<Instruction>()` | 136 bytes | 112 bytes | -24 bytes | The same two revisions | The same command |
| `size_of::<StackSlot>()` | 40 bytes | 32 bytes | -8 bytes | The same two revisions | The same command |
| Peak live bytes of one Terra scene | 24096 bytes | 20192 bytes | -3904 bytes, or -16.20 percent | The same two revisions | `mise run bench-live-bytes`, 30 runs of each revision, spread 0, load 6.13 to 6.76 |
| Terra first frame, median | 0.37 s | 0.37 s | Ratio 1.000 | The same two revisions | 22 before runs and 23 after runs with a one-minute load at or below 12, one before run and one after run next to each other in each round; the load of the 60 runs is 3.23 to 31.34, and 15 runs are above the bound |
| Terra key latency, the medians of the four keys | 0.022 s, 0.025 s, 0.022 s, 0.022 s | 0.021 s, 0.022 s, 0.023 s, 0.022 s | Ratios 0.955, 0.880, 1.045, 0.978 | The same two revisions | The same 22 and 23 runs; the key latency has a resolution of 1 ms, so a key median carries that step |
| Deep copy of a 102400-byte binary | 1748.834 ns, 1632.166 ns, 1640.000 ns, 1630.000 ns in four pairs | 1812.000 ns, 1750.250 ns, 1745.000 ns, 1747.916 ns in the same four pairs | +3.6 to +7.2 percent in every pair | The same two revisions | `mise run bench-term`, one before invocation and one after invocation next to each other in one quiet window, each value the minimum of nine calls inside one test process |
| `arc_clone_drop_binary_102400` | 9.952 ns, 9.952 ns, 9.939 ns | 9.612 ns, 9.609 ns, 9.612 ns | About -3.4 percent | The same two revisions | The same command and the same quiet window |
| Read of the size of a 102400-byte binary | 0.813 ns, 0.814 ns, 0.813 ns | 0.878 ns, 0.877 ns, 0.877 ns | +0.064 ns, or +7.9 percent | The same two revisions | The same command, the minimum of nine timed harness calls, load 7.88 to 8.48, three rounds of each revision |
| Local `call_only` floor loop, 200000 iterations | 120.7 ns | 117.9 ns | -2.8 ns | The same two revisions | `mise run bench-dispatch`, load `{ 6.45 6.73 4.81 }` for both runs; the pinned ERTS number of the same session is 2.67 ns |
| `call_ext_only` floor loop, 200000 iterations | 274.3 ns | 250.8 ns | -23.5 ns | The same two revisions | The same run; the pinned ERTS number is 2.81 ns |
| Local `call_only` plus `gc_bif '+'` | 189.7 ns | 187.9 ns | -1.8 ns | The same two revisions | The same run; the pinned ERTS number is 2.65 ns |
| Local `call_only` plus the `bif` of `tuple_size/1` | 191.6 ns | 186.9 ns | -4.7 ns | The same two revisions | The same run; the pinned ERTS number is 3.34 ns |
| Local `call_only` plus the `bif` of `element/2` | 195.4 ns | 190.7 ns | -4.7 ns | The same two revisions | The same run; the pinned ERTS number is 2.61 ns |
| Local `call_only` plus the `bif` of `hd/1` | 194.6 ns | 190.8 ns | -3.8 ns | The same two revisions | The same run; the pinned ERTS number is 2.62 ns |
| Local `call_only` plus the `bif` of `map_get/2` on a 1000-key map | 261.2 ns | 251.1 ns | -10.1 ns | The same two revisions | The same run; the pinned ERTS number is 7.53 ns |

Source: the body and the verification of [#598](https://github.com/fahchen/pbeam/pull/598). The gate of row 6 is a term of at most 16 bytes with at least a 10 percent end-to-end Terra gain, or at least a 20 percent reduction of peak live bytes with no time regression above 10 percent. The gate fails on its numbers, and the owner decided to land the compact term as the first half of the target. The change is landed, and this entry states the measured effect of the compact term.

### 2026-10-14 06:06 JST - Bounded scheduler release, task [#559](https://github.com/fahchen/pbeam/issues/559), pull request [#578](https://github.com/fahchen/pbeam/pull/578), row 11 of the term plan

The scheduler owns a release work list, and the exit of a process moves its containers to that list as containers. One call of `Scheduler::drain_release` processes at most `RELEASE_BATCH`, which is 256 units, in one scheduler step; a unit is one term, one element of an element array, one pair of a map, one register or Y slot, one dictionary pair, one mailbox entry, one monitor, one frame, or one step of a map walk, and a unit pushes at most two terms. The exit of a process therefore no longer releases its whole term graph in one step.

| Shape, maximum pause of one scheduler step | Before `3b3a21d` | After `f899ea91` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| List of 1000000 cells, process exit | 23664.417 µs | 56.541 µs | -23607.876 µs | Before `3b3a21d`, after `f899ea91` | `mise run bench-release`, the maximum of five runs of one path; before load `{ 4.95 45.12 66.44 }` to `{ 4.64 44.39 66.06 }`, after load `{ 6.23 51.10 69.48 }` to `{ 5.45 48.73 68.31 }` |
| Chain of 1000000 levels, process exit | 26319.916 µs | 48.834 µs | -26271.082 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| Skewed chain of 200000 levels, process exit | 6594.000 µs | 42.000 µs | -6552.000 µs | The same two revisions | The same run of each revision, at the before and after loads above; the exit rows of this shape after the change ran at load `{ 5.57 49.48 68.69 }` to `{ 5.45 48.73 68.31 }` |
| Tuple tree of 1048575 cells, process exit | 20815.709 µs | 55.208 µs | -20760.501 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| Flat tuple of 1000000 elements, process exit | 3713.584 µs | 108.208 µs | -3605.376 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| Map of 20000 keys, process exit | 609.833 µs | 15.542 µs | -594.291 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| Dictionary of 100000 entries, process exit | 442.000 µs | 13.875 µs | -428.125 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| Mailbox of 100000 messages, process exit | 561.792 µs | 7.584 µs | -554.208 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| Queued exit signal with a list of 1000000 cells, process exit | 24748.041 µs | 122.166 µs | -24625.875 µs | The same two revisions | The same run of each revision, at the before and after loads above |
| List of 1000000 cells, shutdown | 23696.708 µs | 24979.333 µs | Stays synchronous | The same two revisions | The same run of each revision, at the before and after loads above; the drop of the `Scheduler` at the end of a run stays synchronous |

Source: the measurement section of [#578](https://github.com/fahchen/pbeam/pull/578). Limits of that source: the control row does not agree between the two revisions for the map and the queued signal, so the control does not prove the size of every row; the pause is the pause of one scheduler step of the release path and not a wall-clock deadline; `Held bytes` is not the peak allocation of the whole process; a later clean run of the harness at `2794bfd` recorded exit rows of 15.708 µs to 100.875 µs at load `{ 8.73 12.74 16.71 }` to `{ 11.23 13.47 17.05 }`.

### 2026-10-14 05:35 JST - The interleaved check of the 120 ns gate of the record, task [#599](https://github.com/fahchen/pbeam/issues/599), pull request [#600](https://github.com/fahchen/pbeam/pull/600)

The change records the interleaved check of the row 9 gate of the term plan, at most 120 ns for each iteration, in `docs/interpreter-dispatch-measurements.md`. The session interleaves the after-revision of [#585](https://github.com/fahchen/pbeam/pull/585) (`d54e6a4`) with the head of `origin/main` at the time (`ddb32f6e`, which holds [#582](https://github.com/fahchen/pbeam/pull/582), [#589](https://github.com/fahchen/pbeam/pull/589), [#590](https://github.com/fahchen/pbeam/pull/590), and [#597](https://github.com/fahchen/pbeam/pull/597)). Every round runs `mise run bench-dispatch` in one detached worktree for each revision.

| Round | `d54e6a4` local / position, ns | `ddb32f6e` local / position, ns | One-minute load, `d54e6a4` / `ddb32f6e` |
| --- | --- | --- | --- |
| 1 | 124.9 / 124.1 | 119.1 / 119.1 | 14.88 / 26.15 |
| 2 | 118.4 / 118.4 | 117.9 / 117.8 | 32.26 / 29.92 |
| 3 | 118.5 / 118.4 | 119.8 / 118.2 | 27.68 / 25.70 |
| Minimum | 118.4 / 118.4 | 117.9 / 117.8 | Not applicable to a minimum |

| Shape | `d54e6a4` | `ddb32f6e` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only` floor loop, 200000 iterations, minimum of the three rounds | 118.4 ns | 117.9 ns | No difference claimed | `d54e6a4` against `ddb32f6e` | Three interleaved rounds in one session; each number is the minimum of three rounds, and each round is the minimum of nine probe runs inside one invocation of `mise run bench-dispatch` |
| Position `call_only` floor loop, 200000 iterations, minimum of the three rounds | 118.4 ns | 117.8 ns | No difference claimed | The same two revisions | The same three rounds |
| `call_ext_only` floor loop, 200000 iterations, minimum of the three rounds | 285.7 ns | 286.4 ns | +0.7 ns, the session control | The same two revisions | The same three rounds |

Source: the section "The interleaved check of the 120 ns gate of the term plan" of `docs/interpreter-dispatch-measurements.md` and the body of [#600](https://github.com/fahchen/pbeam/pull/600). The three rounds do not establish the size of any difference between the two revisions, and the gate claim rests on the minimum across the rounds. Both revisions pass the gate at that minimum, and both pass in rounds 2 and 3. Round 1 does not pass for `d54e6a4`, which measures 124.9 ns and 124.1 ns there. This session does not re-measure the earlier baseline of step 9 of the record, so it neither confirms nor denies the +15.6 ns layout effect that the step 9 session reported for the control.

### 2026-10-14 05:13 JST - Same-register map update, task [#554](https://github.com/fahchen/pbeam/issues/554), pull request [#597](https://github.com/fahchen/pbeam/pull/597)

`put_map_assoc` and `put_map_exact` change the map of the source register in place when the source and the destination name one register and the process holds the only `Arc` of the map. Every other case copies the form of the map, as before.

| Map keys | Owned path, ns for one operation | Retained-version path, ns for one operation | Experiment target | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| 1 | 147.92 | 207.08 | - | `b0232d75` | `mise run bench-term`, one thread, each number the minimum of nine timed runs of 200 rounds; load `{ 20.52 24.13 34.96 }` |
| 10 | 407.70 | 540.21 | - | `b0232d75` | The same run |
| 100 | 119.58 | 375.00 | - | `b0232d75` | The same run |
| 1000 | 125.00 | 430.00 | The owned path below 300 ns and the retained-version path below 600 ns | `b0232d75` | The same run |

Source: the verification of [#597](https://github.com/fahchen/pbeam/pull/597). The two columns are the two code paths at the merged revision, and the pull request does not re-measure the base revision. The table is therefore not a before-and-after pair across two revisions, and this entry does not state one. The change routes the same-register single-owner case to the owned path; a map that another owner holds is never written in place, and `Map::update` still copies a shared node through `Arc::make_mut`.

### 2026-10-14 04:05 JST - Byte-counting live-byte harness, task [#587](https://github.com/fahchen/pbeam/issues/587), pull request [#589](https://github.com/fahchen/pbeam/pull/589)

The change adds the live-byte walk `src/term/live_bytes.rs`, the public method `BeamProgram::execute_sampled`, and the integration harness `tests/integration/term_live_bytes.rs`, which holds one scene of the Terra Counter target and reports the peak live bytes of the guest containers. The walk counts the runtime containers and installs no allocator, because `AGENTS.md` forbids new `unsafe` code.

| Shape | Before | After | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Peak live bytes of one Terra scene | Not measured, because the harness does not exist before this change | 24096 bytes | This is the baseline of the harness | `ffb1a05a`, the harness head | Two runs, spread 0, load `{ 41.79 77.93 80.63 }` and `{ 38.77 76.71 80.18 }` |
| Baseline live bytes at the first sample | Not measured | 40 bytes | The first sample of the walk | `ffb1a05a` | The same two runs |
| Peak resident set size of the same run | Not measured | 3888 KB | Supporting evidence only | `ffb1a05a` | The same two runs; the number is not the live guest bytes |

Source: the verification of [#589](https://github.com/fahchen/pbeam/pull/589). The live-byte number is a deterministic count of the runtime containers, and the two runs agree with a spread of 0 bytes. The harness holds one scene of the Terra target and not the full Terra Counter application, so the peak is the maximum at a scheduling step and the harness sees no peak inside one turn.

### 2026-10-14 02:48 JST - The host-term boundary, task [#579](https://github.com/fahchen/pbeam/issues/579), pull request [#582](https://github.com/fahchen/pbeam/pull/582)

`AtomTable` carries the identity of its program, and `HostTerm { term, atoms }` pairs a term with the atom table that owns each of its atoms at any depth. The three public entry points take `arguments: &[HostTerm]` and return `Result<HostTerm, BeamError>`, and the import between two runs rejects every local fun.

| Item | Before | After | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| `size_of::<Term>()` | 40 bytes | 40 bytes | 0 | `5e73dde4` | Pinned rustc 1.99.0; the type size carries no load |
| Entry-point argument | `&[Term]` | `&[HostTerm]` | Public API | `5e73dde4` | `mise run check` at load `{ 12.97 9.48 10.17 }` |
| Entry-point result | `Result<Term, BeamError>` | `Result<HostTerm, BeamError>` | Public API | `5e73dde4` | The same check |
| `BeamProgram::atoms` | `AtomTable` | `Arc<AtomTable>` | Public API | `5e73dde4` | The same check |
| Import walk of a term that holds one subtree twice | One allocation for each occurrence | One allocation, shared | Fewer allocations | `5e73dde4` | The same check |
| Comparison of a term that holds one subtree twice | One visit for each occurrence | One visit for each pair of storage | Fewer visits | `5e73dde4` | The same check |

Source: the verification of [#582](https://github.com/fahchen/pbeam/pull/582). The pull request claims no timing number, because the change adds the run-table retention, the import memory, and the comparison memory to the run boundary, and the interpreter loop and the `Term` layout are unchanged.

### 2026-10-14 01:50 JST - Resolved jump and frame target in a field, task [#542](https://github.com/fahchen/pbeam/issues/542), pull request [#585](https://github.com/fahchen/pbeam/pull/585)

`ModuleTables::build` resolves each branch label and each local-call label of a module one time in a second pass over the instruction stream, and writes the resolved code position into the instruction. The run loop then reads a field and runs no lookup in `jump_targets` or `function_starts`, and `size_of::<Instruction>()` stays 136 bytes.

| Shape | Before `ea3e28a` | After `d54e6a4` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only`, 200000 iterations | 127.5 ns | 115.9 ns | -11.6 ns, or -9.1 percent | Before `ea3e28a`, after `d54e6a4` | Ten interleaved runs of each revision in two sessions; each cell is the minimum of the ten runs, and each run is the minimum of nine probe runs; the one-minute load of the twenty runs is 4.11 to 7.31 |
| Position `call_only`, 200000 iterations | 127.4 ns | 115.9 ns | -11.5 ns, or -9.0 percent | The same two revisions | The same twenty runs |
| Local `call_only`, 20000 iterations | 129.3 ns | 117.7 ns | -11.6 ns | The same two revisions | The same twenty runs |
| Position `call_only`, 20000 iterations | 129.2 ns | 117.7 ns | -11.5 ns | The same two revisions | The same twenty runs |
| `call_ext_only`, 200000 iterations, the control | 264.2 ns | 279.8 ns | +15.6 ns | The same two revisions | The same twenty runs; a layout effect of the run loop of this binary, and this change does not own the external call path |
| `call_ext_only`, 20000 iterations, the control | 264.4 ns | 281.7 ns | +17.3 ns | The same two revisions | The same twenty runs |

Source: the verification of [#585](https://github.com/fahchen/pbeam/pull/585). The benchmark holds no taken `Jump` and no taken `fail_label` in a loop, so the jump half of the change carries no number from `mise run bench-dispatch`.

### 2026-10-14 00:23 JST - Operation-specific operand forms, row 9 of the term plan, task [#560](https://github.com/fahchen/pbeam/issues/560), pull request [#576](https://github.com/fahchen/pbeam/pull/576)

The loader turns `test is_eq_exact` into `Instruction::TestExactEqualOperands` when both operands are a register or an immediate term and at least one operand is a register. The run loop then reads a checked register slot or the stored `Term`, and it matches no `Operand` for that instruction.

| Shape | Before `e0c4c81` | After `3b464b1` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only`, 200000 iterations | 141.1 ns | 138.4 ns | -2.7 ns | Before `e0c4c81`, after `3b464b1` | Six interleaved runs of each revision in one session; each cell is the minimum of the six runs, and each run is the minimum of nine probe runs; the load of round 1 is `{ 11.97 24.72 24.15 }` before and `{ 9.64 23.36 23.68 }` after, and the load of round 6 is `{ 3.24 16.69 21.00 }` before and `{ 3.35 16.94 21.12 }` after |
| Position `call_only`, 200000 iterations | 141.1 ns | 135.9 ns | -5.2 ns | The same two revisions | The same six rounds |
| Local `call_only`, 20000 iterations | 143.5 ns | 140.1 ns | -3.4 ns | The same two revisions | The same six rounds |
| Position `call_only`, 20000 iterations | 145.8 ns | 139.8 ns | -6.0 ns | The same two revisions | The same six rounds |
| `call_ext_only`, 200000 iterations | 261.6 ns | 263.2 ns | +1.6 ns | The same two revisions | The same six rounds; inside the run-to-run noise of that shape, and this change does not touch the external call path |
| Added `gc_bif '+'`, 200000 iterations | 72.9 ns | 70.5 ns | -2.4 ns | The same two revisions | The same six rounds |
| Added `tuple_size/1` | 72.2 ns | 70.0 ns | -2.2 ns | The same two revisions | The same six rounds |
| Added `element/2` | 75.8 ns | 73.3 ns | -2.5 ns | The same two revisions | The same six rounds |
| Added `hd/1` | 73.3 ns | 72.6 ns | -0.7 ns | The same two revisions | The same six rounds |
| Added `map_get/2` on a 1000-key map | 126.8 ns | 122.8 ns | -4.0 ns | The same two revisions | The same six rounds |

Source: the verification of [#576](https://github.com/fahchen/pbeam/pull/576). The measured fall is 2.7 ns to 5.2 ns for each iteration, below the 24.1 ns that the issue records as the gap to the 120 ns row gate, so the pull request claims no pass of that gate. The session control is the base local `call_only` floor loop of the same session at 141.1 ns to 145.9 ns.

### 2026-10-13 23:53 JST - ETS set key index, task [#543](https://github.com/fahchen/pbeam/issues/543), pull request [#570](https://github.com/fahchen/pbeam/pull/570)

The change adds `KeyIndex`, an open-addressing table with linear probing, beside the object slots of an ETS `set` table. It stores the ERTS stored hash of each key and the slot id of its object, so a lookup reads a slot instead of walking every object.

| Operation | Before `3b3a21d` | After `0c27d4d` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| A `set` lookup, 1 object | 5.000 ns | 9.790 ns | +95.8 percent | Before `3b3a21d`, after `0c27d4d` | `cargo test --release --lib data_structure_costs`; three runs of each revision, one cell is the minimum of the runs and each run is the minimum of nine timed runs; load before `{ 4.64 6.07 6.76 }`, after `{ 11.75 22.48 17.40 }` |
| A `set` lookup, 10 objects | 43.125 ns | 9.580 ns | -77.8 percent | The same two revisions | The same micro-measurement |
| A `set` lookup, 100 objects | 419.170 ns | 12.085 ns | -97.12 percent | The same two revisions | The same micro-measurement |
| A `set` lookup, 1000 objects | 4195.205 ns | 10.000 ns | -99.76 percent | The same two revisions | The same micro-measurement |
| A `set` lookup that misses, 1000 objects | 4191.875 ns | 5.205 ns | -99.88 percent | The same two revisions | The same micro-measurement |
| An insert of one key, 1 to 201 objects | 434.375 ns | 39.165 ns | -90.98 percent | The same two revisions | The same micro-measurement |
| An insert of one key, 1000 to 1200 objects | 4703.330 ns | 45.205 ns | -99.04 percent | The same two revisions | The same micro-measurement |
| A delete of one key, 201 to 1 objects | 55.210 ns | 33.125 ns | -40.00 percent | The same two revisions | The same micro-measurement |
| A delete of one key, 1200 to 1000 objects | 4273.125 ns | 33.960 ns | -99.21 percent | The same two revisions | The same micro-measurement |
| `ets_probe:lookup_loop/0`, 1000 inserts and 2000 lookups | 9988500 ns | 3315083 ns | -66.8 percent | The same two revisions | The compiled program, the least of nine timed runs; load before `{ 10.76 11.51 10.06 }`, after `{ 10.56 21.87 17.25 }` |
| `ets_probe:insert_loop/0`, 1000 inserts | 3220333 ns | 984625 ns | -69.4 percent | The same two revisions | The same program run |

Source: the verification of [#570](https://github.com/fahchen/pbeam/pull/570). The +95.8 percent row at one object is a known limitation of the change, and the interpreter work of the loop bounds the after time of the two program rows, so the program gain is below the gain of one operation.

### 2026-10-13 22:52 JST - The frame module and the execution cursor, task [#550](https://github.com/fahchen/pbeam/issues/550), pull request [#569](https://github.com/fahchen/pbeam/pull/569)

The change removes the per-iteration comparison of the active module from the run loop. The loop no longer reads the active frame on each iteration, and each exit that can change the active module refreshes the cursor: a call that enters a frame, a return, and an exception that truncates the stack.

| Shape | Before `a0104b4` | After `3b3a21d2` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only` floor loop, 200000 iterations | 147.7 ns | 144.1 ns | -3.6 ns | Before `a0104b4`, after `3b3a21d2` | `mise run bench-dispatch`, six interleaved runs of each build in two sessions; each cell is the minimum of the six runs, and each run is the minimum of nine probe runs; load 2.79 to 11.02 |
| Position `call_only` floor loop, 200000 iterations | 147.4 ns | 143.8 ns | -3.6 ns | Before `a0104b4`, after `3b3a21d2` | The same runs |
| Local `call_only` floor loop, 20000 iterations | 146.9 ns | 143.0 ns | -3.9 ns | Before `a0104b4`, after `3b3a21d2` | The same runs |
| Position `call_only` floor loop, 20000 iterations | 147.6 ns | 143.5 ns | -4.1 ns | Before `a0104b4`, after `3b3a21d2` | The same runs |
| `call_ext_only` floor loop, 200000 iterations | 283.3 ns | 262.3 ns | -21.0 ns | Before `a0104b4`, after `3b3a21d2` | The same runs |
| `call_ext_only` floor loop, 20000 iterations | 283.1 ns | 260.1 ns | -23.0 ns | Before `a0104b4`, after `3b3a21d2` | The same runs |
| Attribution `L`, local `call_only`, 200000 iterations | 148.586 ns | 145.197 ns | -3.389 ns | Before `a0104b4`, after `3b3a21d2` | `bash scripts/dispatch_attribution.sh`, three builds interleaved in one session, six runs of each build, the minimum of six runs; load `{ 4.13 9.40 11.99 }` to `{ 4.42 9.90 12.24 }` |
| Attribution `L` of the `V2` ablation, the control | 148.586 ns | 144.596 ns | -3.990 ns | Before `a0104b4`, after the `V2` ablation build | The same three builds |
| Micro-measurement of the module identity comparison | 0.495 ns for one operation | Removed from the loop | -0.495 ns for one operation | The task branch head `af039313`, merged as `3b3a21d2` | The minimum of nine timed runs of 100000 operations; load `{ 4.13 ... }` to `{ 10.31 ... }` |

Source: the verification of [#569](https://github.com/fahchen/pbeam/pull/569) and the section "The active module and the execution cursor" of `docs/interpreter-dispatch-measurements.md`. A second session of the same revision gives a fall of 4.115 ns on the attribution `L`, and the pull request states that both falls are below the 4.591 ns of half the historical share at `c2f0341` and above half of the re-measured share at `a0104b4`.

### 2026-10-13 09:23 JST - Term equality without the pair-stack allocation, task [#541](https://github.com/fahchen/pbeam/issues/541), pull request [#548](https://github.com/fahchen/pbeam/pull/548)

`Term::eq` compares two terms without a heap allocation for a shallow or a shared comparison. The work stack holds the first 8 pending pairs in an array in the frame of the comparison and every later pair in a heap vector above the array.

| Shape | Before `6d3884d` | After `87a44ab` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| One comparison of two integers | 31.664 ns | 6.647 ns | -79.0 percent | Before `6d3884d`, after `87a44ab` | `cargo test --release --lib data_structure_costs`; three invocations of each revision, one cell is the minimum of the invocations and each invocation is the minimum of nine timed runs; before load `{ 6.79 7.06 10.36 }`, `{ 6.22 6.91 10.23 }`, `{ 6.63 6.97 10.19 }`; after `{ 6.61 7.01 10.30 }`, `{ 6.36 6.93 10.21 }`, `{ 8.07 7.25 10.25 }` |
| One comparison of two atoms | 30.798 ns | 7.589 ns | -75.4 percent | The same two revisions | The same three invocations |
| One comparison of two 1000-element tuples that share one array | 28.335 ns | 5.000 ns | -82.4 percent | The same two revisions | The same three invocations |
| One comparison of two 3-element tuples with equal content | 72.260 ns | 9.881 ns | -86.3 percent | The same two revisions | The same three invocations |
| Local `call_only` floor loop, 200000 iterations | 168.7 ns | 150.2 ns | -11.0 percent | The same two revisions | `mise run bench-dispatch`, three interleaved runs of each revision; before load `{ 8.21 7.50 10.21 }`, `{ 8.59 7.73 10.19 }`, `{ 10.33 8.29 10.28 }`; after `{ 9.00 7.73 10.25 }`, `{ 9.39 7.98 10.22 }`, `{ 11.41 8.66 10.36 }` |
| Position `call_only` floor loop, 200000 iterations | 166.6 ns | 150.2 ns | -9.8 percent | The same two revisions | The same three runs |
| Allocations of one comparison of two integers | 1 | 0 | -1 | The same two revisions | The counting allocator of the harness |
| Allocations of two 1000-element tuples with equal content in different storage | 2 | 1 | -1 | The same two revisions | The same counter |
| Allocations of two 1000-element tuples that share one array | 1 | 0 | -1 | The same two revisions | The same counter |

Source: the verification of [#548](https://github.com/fahchen/pbeam/pull/548). The merged revision is `599a9c06`. The head of the pull request also holds the 8-slot boundary fix `030ff8b`; the boundary fix changes the allocation count of the two 8-element tuples with equal content in different storage from 2 at `6d3884d` to 0 at `030ff8b`. Known limits: +4.0 percent on a 1000-element tuple walk, +1.8 percent on a 1000-key map walk, and no isolated cause for the `map_get/2` delta.

### 2026-10-12 - Dispatch step 9, combine the measured instruction pairs, task [#517](https://github.com/fahchen/pbeam/issues/517), pull request [#538](https://github.com/fahchen/pbeam/pull/538) - Not landed

The closed pull request [#538](https://github.com/fahchen/pbeam/pull/538) adds `Instruction::TestCompareBif` and `Instruction::BifCall`, so the measured `test` with `gc_bif` shape and the measured `gc_bif` with `call_only` shape each run in one dispatch step. The gate of the step fails, so the task closed without an implementation and the pull request closed without a merge.

| Shape | Before `c6cfb3a` | After `945e9f8` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only`, 200000 iterations | 163.2 ns | 165.7 ns | +2.5 ns | Before `c6cfb3a`, after `945e9f8` | Three interleaved runs of each revision; each number is the minimum of the three runs, and each run is the minimum of nine probe runs; the loads of the three before runs are `{ 11.99 17.10 18.04 }`, `{ 9.35 15.80 17.52 }`, and `{ 11.96 15.60 17.36 }`, and of the three after runs `{ 10.46 16.43 17.77 }`, `{ 8.96 15.30 17.30 }`, and `{ 10.07 14.93 17.08 }` |
| Position `call_only`, 200000 iterations | 163.5 ns | 165.4 ns | +1.9 ns | The same two revisions | The same three runs |
| `call_ext_only`, 200000 iterations | 290.1 ns | 288.1 ns | -2.0 ns | The same two revisions | The same three runs |
| Added `gc_bif '+'` | 70.7 ns | 81.0 ns | +10.3 ns | The same two revisions | The same three runs |
| Added `tuple_size/1` | 69.6 ns | 81.7 ns | +12.1 ns | The same two revisions | The same three runs |
| Added `element/2` | 74.4 ns | 83.5 ns | +9.1 ns | The same two revisions | The same three runs |
| Added `hd/1` | 75.1 ns | 84.5 ns | +9.4 ns | The same two revisions | The same three runs |
| Added `map_get/2` on a 1000-key map | 131.6 ns | 143.2 ns | +11.6 ns | The same two revisions | The same three runs |

Source: the closing comment on [#517](https://github.com/fahchen/pbeam/issues/517) and the verification of [#538](https://github.com/fahchen/pbeam/pull/538). The gate is at most 150 ns for each iteration of the local `call_only` floor loop, and the measured value is 165.7 ns. The pairs kept every exit exact against the pinned ERTS, including the reduction total of 4399 on both runs. The change is marked not landed, and it is not a measured gain of any step.

### 2026-10-12 21:47 JST - Dispatch step 7, remove the drop allocations, task [#515](https://github.com/fahchen/pbeam/issues/515), pull request [#535](https://github.com/fahchen/pbeam/pull/535)

`Map::repr` becomes an `Option<Arc<Repr>>`, and the drain takes the form with `Option::take` instead of building a placeholder form. `Term::drop` takes the storage of the term first and gives it to the walk, so a term that gives no nested term builds no work vector.

| Shape | Before `281483c` | After `20b8f44` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Drop of an immediate | 4.06 ns | 2.45 ns | -1.61 ns | Before `281483c`, after `20b8f44` | `mise run bench-drop`, release; three runs of each revision, each number is the minimum of nine timed runs; before load `{ 4.16 7.69 12.02 }`, `{ 4.30 7.60 11.94 }`, `{ 4.09 7.39 11.78 }`; after `{ 4.07 7.61 11.96 }`, `{ 4.12 7.51 11.88 }`, `{ 4.09 7.33 11.74 }` |
| Drop of a tuple of three elements, last holder | 64.70 ns | 32.06 ns | -32.64 ns | The same two revisions | The same three runs |
| Drop of a tuple of three elements, shared holder | 5.99 ns | 3.18 ns | -2.81 ns | The same two revisions | The same three runs |
| Drop of a tuple of one element, last holder | 53.87 ns | 24.24 ns | -29.63 ns | The same two revisions | The same three runs |
| Drop of a tuple of one element, shared holder | 5.99 ns | 3.19 ns | -2.80 ns | The same two revisions | The same three runs |
| Drop of a map of one key, last holder | 91.10 ns | 67.08 ns | -24.02 ns | The same two revisions | The same three runs |
| Drop of a map of one key, shared holder | 25.91 ns | 6.57 ns | -19.34 ns | The same two revisions | The same three runs |
| Drop of a list of 1000000 cells | 24.69 ms | 22.29 ms | -2.41 ms | The same two revisions | The same command; three runs only for this shape, and the report gives 22.758750 ms, 22.288333 ms, and 22.757625 ms for each after run |
| Allocations of the drop of a map of one key, shared holder | 1 | 0 | -1 | The same two revisions | `cargo test --release --test drop_allocations`, the minimum of the three runs of each revision |
| Allocations of the drop of a tuple of three elements, last holder | 1 | 0 | -1 | The same two revisions | The same counter |
| `get` on a 1000-key map | 68.57 ns | 68.46 ns | -0.11 ns | The same two revisions | `cargo test --release --lib records_the_map_operation_cost`; three runs, each the minimum of nine runs of 200 rounds; before load `{ 3.55 7.00 11.52 }` to `{ 7.42 7.51 11.54 }`, after `{ 3.34 6.90 11.46 }` to `{ 7.13 7.44 11.46 }` |
| `put` on a copy of the 1000-key map | 483.75 ns | 476.67 ns | -7.08 ns | The same two revisions | The same map run |
| `put_map_exact` on a copy of the 1000-key map | 444.58 ns | 436.25 ns | -8.33 ns | The same two revisions | The same map run |

Source: the verification of [#535](https://github.com/fahchen/pbeam/pull/535). The gate of the step is no measured regression on a term drop, and it holds for every measured shape. The map limb of the plan is at or below the tuple drop cost plus 3 ns; the shared map is 0.4 ns to 0.6 ns above it, because the map arm of `Term::drop` builds the walk vector and calls `Map::drain_into` where the tuple arm takes its array in one call.

### 2026-10-12 20:30 JST - Dispatch step 6, measure the decoded instruction form, task [#514](https://github.com/fahchen/pbeam/issues/514), pull request [#527](https://github.com/fahchen/pbeam/pull/527)

The step measures the decoded instruction form and changes no runtime behavior, so the before numbers and the after numbers are the same probes at the same revision. The decision of the step follows the step 8 gate and keeps the decoded form, and step 8 is skipped.

| Item | Before | After | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| `size_of::<Instruction>()` | 136 bytes | 136 bytes | 0 | `ea4dd13` | Pinned rustc 1.99.0, `-Zprint-type-sizes`; a type size carries no load |
| `size_of::<Operand>()` | 40 bytes | 40 bytes | 0 | `ea4dd13` | The same command |
| `size_of::<Term>()` | 40 bytes | 40 bytes | 0 | `ea4dd13` | The same command |
| `size_of::<BifId>()` | 4 bytes | 4 bytes | 0 | `ea4dd13` | The same command |
| Retired instructions of one floor-loop iteration | 4 | 4 | 0 | `ea4dd13` | `records_the_decoded_loop_body` in `tests/runtime/interpreter.rs` |
| Instruction-slot bytes of one iteration | 544 bytes inside an 816-byte range | 544 bytes inside an 816-byte range | 0 | `ea4dd13` | The same test and `FLOOR_LOOP_RECORDS` |
| Memory-system proxy, a 136-byte record against a 32-byte record at the estimated Terra stream size | Not measured | 2.09 ns for each record, or 8.38 ns for each iteration, which is 5.0 percent of the 167.4 ns loop | An estimate, not a measurement | `ea4dd13` | The proxy runs as its own step of the benchmark script, and each number is the minimum of five runs |

Source: the verification of [#527](https://github.com/fahchen/pbeam/pull/527), the section "Step 6" of `docs/interpreter-dispatch-measurements.md`, and section 2 of `docs/interpreter-dispatch-plan.md`. The 5.0 percent is a share of one floor-loop iteration and not a Terra redraw gain. The L1 cache residency of the loop is an assumption, and the host has no hardware counter to confirm it.

### 2026-10-12 - Dispatch step 8, the prepared instruction form, task [#516](https://github.com/fahchen/pbeam/issues/516) - Not landed

The step would add a prepared instruction form with a constant pool and the X register array of `max_x`. The step is skipped because its gate fails.

| Gate | Value | Result | Revision | Load or run rule | Source |
| --- | --- | --- | --- | --- | --- |
| At least a 10 percent gain on the floor loop | The memory-system proxy gives 5.0 percent for each iteration | Below the gate | `ea4dd13` | The proxy runs alone, the minimum of five runs | [#527](https://github.com/fahchen/pbeam/pull/527) and the record |
| At most 32 bytes for each instruction | The decoded form is 136 bytes and the loop reads 544 bytes of slots in an 816-byte range | The decoded form is kept | `ea4dd13` | The layout test at the pinned rustc | The plan and the record |

Source: section 2 of `docs/interpreter-dispatch-plan.md` and the section "What comes next" of `docs/interpreter-dispatch-measurements.md`. The step number does not change, and step 9 follows step 7 without step 8. The step is marked not landed, and it is not a measured gain.

### 2026-10-12 19:30 JST - Dispatch step 5, the borrowed fixed-arity BIF arguments, task [#513](https://github.com/fahchen/pbeam/issues/513), pull request [#526](https://github.com/fahchen/pbeam/pull/526)

A `bif` instruction no longer builds a `Vec<Term>` for its arguments. It reads the argument operands into `BifArguments`, which holds at most the three terms of a `gc_bif3` instruction on the stack, and it passes the borrowed slice to the BIF body.

| Shape | Before `1e86242` | After `8385d12` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Added `gc_bif '+'`, 200000 iterations | 107.2 ns | 77.7 ns | -29.5 ns, or -27.5 percent | Before `1e86242`, after `8385d12` | Three interleaved runs of each revision; each cell is the minimum of the three runs, and each run is the minimum of nine probe runs; the before runs had a load of 1.96 to 2.68 and the after runs a load of 23.32 to 27.02 |
| Added `tuple_size/1` | 82.7 ns | 81.8 ns | -0.9 ns, or -1.1 percent | The same two revisions | The same three runs; inside the run-to-run noise, and the step claims no gain |
| Added `element/2` | 95.8 ns | 83.0 ns | -12.8 ns, or -13.4 percent | The same two revisions | The same three runs |
| Added `hd/1` | 84.5 ns | 78.5 ns | -6.0 ns, or -7.1 percent | The same two revisions | The same three runs |
| Added `map_get/2` on a 1000-key map | 164.1 ns | 155.5 ns | -8.6 ns, or -5.2 percent | The same two revisions | The same three runs |
| Local `call_only` floor loop, 200000 iterations | 171.0 ns | 166.7 ns | -4.3 ns, or -2.5 percent | The same two revisions | The same three runs; the movement is inside the noise, and the step claims no reduction for the floor shapes |
| Position `call_only` floor loop, 200000 iterations | 170.9 ns | 167.0 ns | -3.9 ns, or -2.3 percent | The same two revisions | The same three runs; no reduction claimed |
| `call_ext_only` floor loop, 200000 iterations | 296.8 ns | 292.0 ns | -4.8 ns, or -1.6 percent | The same two revisions | The same three runs; no reduction claimed |
| Added allocations of `gc_bif '+'`, `tuple_size/1`, `element/2`, and `hd/1` | 1 | 0 | -1 | The same two revisions | The counting allocator of `tests/integration/bif_allocations.rs` |
| Allocations of the `call_only` floor loop | 2 | 1 | -1 | The same two revisions | The same counter; the step does not own this row |

Source: the verification of [#526](https://github.com/fahchen/pbeam/pull/526). The gate of the step is a `+` delta below 80 ns, and the three after runs give 77.7 ns, 78.8 ns, and 78.2 ns. The after runs had the higher load, so the after floor numbers are a floor and not a best case.

### 2026-10-12 19:02 JST - Dispatch step 4, the BIF identity at load time, task [#512](https://github.com/fahchen/pbeam/issues/512), pull request [#525](https://github.com/fahchen/pbeam/pull/525)

The loader resolves the MFA of a `bif` import and of a `call_ext` import to a `BifId` one time. A `bif` instruction holds no module name and no function name, and a `call_ext` of a BIF reaches the body with no binary search of the 553-entry `BIFS` table.

| Shape | Before `516ceb1` | After `d244d8f` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only`, 200000 iterations | 249.6 ns | 177.9 ns | -71.7 ns | Before `516ceb1`, after `d244d8f` | Four interleaved runs of `d244d8f`; the before values are the minimum of thirteen interleaved runs at `516ceb1`; each cell is the minimum of the runs, and each run is the minimum of nine probe runs; the load of the fixed head is 8.64 to 12.67 |
| Position `call_only`, 200000 iterations | 249.8 ns | 176.9 ns | -72.9 ns | The same two revisions | The same run groups |
| `call_ext_only`, 200000 iterations | 499.9 ns | 303.5 ns | -196.4 ns | The same two revisions | The same run groups |
| Local `call_only`, 20000 iterations | 251.6 ns | 179.8 ns | -71.8 ns | The same two revisions | The same run groups |
| Position `call_only`, 20000 iterations | 251.5 ns | 179.7 ns | -71.8 ns | The same two revisions | The same run groups |
| `call_ext_only`, 20000 iterations | 501.9 ns | 304.1 ns | -197.8 ns | The same two revisions | The same run groups |
| Added `gc_bif '+'` | 177.7 ns | 105.4 ns | -72.3 ns | The same two revisions | The same run groups |
| Added `tuple_size/1` | 174.9 ns | 85.5 ns | -89.4 ns | The same two revisions | The same run groups |
| Added `element/2` | 179.8 ns | 100.5 ns | -79.3 ns | The same two revisions | The same run groups |
| Added `hd/1` | 173.3 ns | 87.0 ns | -86.3 ns | The same two revisions | The same run groups |
| Added `map_get/2` on a 1000-key map | 255.5 ns | 177.2 ns | -78.3 ns | The same two revisions | The same run groups |

Source: the verification of [#525](https://github.com/fahchen/pbeam/pull/525). The measured revision of the after column is `d244d8f`, the head of that branch, which holds the runtime code of the fixed head `aa4c0e7`. The review rejected the first head `486c0a5`, because a `call_ext` of module code still fell through to `call_mfa`. The gate of the step is a BIF delta below 200 ns, and each added row is below it. The 196.4 ns that the module-qualified external call loses holds about 72 ns from the `bif` identity of the same loop and about 124 ns from two failed 553-entry binary searches of `resolve`.

### 2026-10-12 17:32 JST - Dispatch step 3, the frame and export name IDs, task [#511](https://github.com/fahchen/pbeam/issues/511), pull request [#524](https://github.com/fahchen/pbeam/pull/524)

A frame holds the `ModuleId` of its module and a `NameId` in the place of two `String` values, and an export key is `(NameId, u32)` in the place of `(String, u32)`. The call path therefore allocates no `String`.

| Shape | Before `c284fa4` | After `8cfab38` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only`, 200000 iterations | 283.3 ns | 249.2 ns | -34.1 ns, or -12.0 percent | Before `c284fa4`, after `8cfab38` | Four interleaved runs of each revision; each cell is the minimum of the four runs, and each run is the minimum of nine probe runs; the before runs had a load of 20.17, 16.77, 13.41, and 7.50, and the after runs 17.54, 14.49, 12.05, and 7.06 |
| Position `call_only`, 200000 iterations | 286.1 ns | 249.5 ns | -36.6 ns, or -12.8 percent | The same two revisions | The same four runs |
| `call_ext_only`, 200000 iterations | 556.7 ns | 500.0 ns | -56.7 ns, or -10.2 percent | The same two revisions | The same four runs |
| Local `call_only`, 20000 iterations | 284.9 ns | 251.4 ns | -33.5 ns, or -11.8 percent | The same two revisions | The same four runs |
| Position `call_only`, 20000 iterations | 286.8 ns | 251.6 ns | -35.2 ns, or -12.3 percent | The same two revisions | The same four runs |
| `call_ext_only`, 20000 iterations | 560.1 ns | 507.6 ns | -52.5 ns, or -9.4 percent | The same two revisions | The same four runs |
| Allocations of one local `call_only` iteration | 4 | 2 | -2 | The same two revisions | The allocation harness of the pull request |
| `String` allocations of one local `call_only` iteration | 2 | 0 | -2 | The same two revisions | The same harness |
| `size_of::<Frame>()` | 88 bytes | 40 bytes | -48 bytes | `8cfab38` | Pinned rustc 1.99.0; the record gives this size |

Source: the verification of [#524](https://github.com/fahchen/pbeam/pull/524) and the section "Step 3" of `docs/interpreter-dispatch-measurements.md`. The two allocations that stay are the pair stack of `Term::eq` and the argument `Vec<Term>` of the `gc_bif` path; step 5 owns the second, and step 11 of the term plan owns the pair stack.

### 2026-10-12 16:56 JST - Dispatch step 2, the execution cursor, task [#510](https://github.com/fahchen/pbeam/issues/510), pull request [#523](https://github.com/fahchen/pbeam/pull/523)

The run loop holds the active module and the next instruction index in a `Cursor`. A straight-line instruction advances the cursor, and a call that enters a frame, a return, an exception that truncates the stack, a yield, and a caught error refresh the cursor. The loop compares the integer `ModuleId` of the frame with the `ModuleId` of the cursor instead of a module name `String`.

| Shape | Before `5bbb261` | After `32930f0` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Local `call_only`, 200000 iterations | 331.3 ns | 284.5 ns | -46.8 ns, or -14.1 percent | Before `5bbb261`, after `32930f0` | Three interleaved runs of each revision; each cell is the minimum of the three runs, and each run is the minimum of nine probe runs; the before runs had a load of `{ 17.67 22.81 15.83 }`, `{ 15.38 22.05 15.68 }`, and `{ 13.77 21.37 15.55 }`, and the after runs `{ 16.89 22.57 15.78 }`, `{ 14.09 21.56 15.58 }`, and `{ 12.94 20.94 15.46 }` |
| Position `call_only`, 200000 iterations | 329.6 ns | 287.2 ns | -42.4 ns, or -12.9 percent | The same two revisions | The same three runs |
| `call_ext_only`, 200000 iterations | 576.9 ns | 559.2 ns | -17.7 ns, or -3.1 percent | The same two revisions | The same three runs |
| Local `call_only`, 20000 iterations | 332.5 ns | 285.2 ns | -47.3 ns, or -14.2 percent | The same two revisions | The same three runs |
| Position `call_only`, 20000 iterations | 331.4 ns | 290.4 ns | -41.0 ns, or -12.4 percent | The same two revisions | The same three runs |
| `call_ext_only`, 20000 iterations | 577.2 ns | 553.6 ns | -23.6 ns, or -4.1 percent | The same two revisions | The same three runs |
| Added `gc_bif '+'` | 226.5 ns | 176.9 ns | -49.6 ns, or -21.9 percent | The same two revisions | The same three runs |
| Added `tuple_size/1` | 209.0 ns | 157.1 ns | -51.9 ns, or -24.8 percent | The same two revisions | The same three runs |
| Added `element/2` | 204.4 ns | 172.6 ns | -31.8 ns, or -15.6 percent | The same two revisions | The same three runs |
| Added `hd/1` | 200.6 ns | 163.7 ns | -36.9 ns, or -18.4 percent | The same two revisions | The same three runs |
| Added `map_get/2` on a 1000-key map | 288.9 ns | 240.2 ns | -48.7 ns, or -16.9 percent | The same two revisions | The same three runs |

Source: the verification of [#523](https://github.com/fahchen/pbeam/pull/523) and the section "Step 2" of `docs/interpreter-dispatch-measurements.md`. A second interleaved sequence at load 34 to 55 agrees on the direction and the size of the gain.

### 2026-10-12 16:00 JST - Dispatch step 1, the loader side tables, task [#509](https://github.com/fahchen/pbeam/issues/509), pull request [#521](https://github.com/fahchen/pbeam/pull/521)

The loader builds `jump_targets` and `function_starts` once for each module, and the interpreter reads them. A jump, a taken fail label, a `catch` label, and each frame creation stop scanning the instruction stream.

| Shape | Before `211a645` | After `c06c513` | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Position `call_only`, 20000 iterations | 498.9 ns | 327.8 ns | -171.1 ns, or -34.3 percent | Before `211a645`, after `c06c513` | Three interleaved runs of each revision; each number is the minimum of nine probe runs of one probe; the before runs had a load of `{ 2.41 4.95 5.69 }`, `{ 3.57 5.07 5.71 }`, and `{ 3.53 4.96 5.66 }`, and the after runs `{ 3.39 5.09 5.73 }`, `{ 3.72 5.05 5.70 }`, and `{ 3.30 4.87 5.62 }` |
| Position `call_only`, 200000 iterations | 505.6 ns | 326.2 ns | -179.4 ns, or -35.5 percent | The same two revisions | The same three runs |
| Local `call_only`, 200000 iterations | 334.2 ns | 326.7 ns | -7.5 ns, or -2.2 percent | The same two revisions | The same three runs |
| Local `call_only`, 20000 iterations | 336.1 ns | 328.8 ns | -7.3 ns, or -2.2 percent | The same two revisions | The same three runs |
| `call_ext_only`, 200000 iterations | 562.6 ns | 577.6 ns | +15.0 ns, or +2.6 percent | The same two revisions | The same three runs; the function label is the first label of the module, where the removed scan read two instructions only, and this step records the rise as a known gap |
| `call_ext_only`, 20000 iterations | 562.9 ns | 577.3 ns | +14.4 ns, or +2.6 percent | The same two revisions | The same three runs |
| Added `gc_bif '+'` | 248.9 ns | 229.2 ns | -19.7 ns | The same two revisions | The same three runs |
| Added `tuple_size/1` | 238.1 ns | 208.6 ns | -29.5 ns | The same two revisions | The same three runs |
| Added `element/2` | 242.7 ns | 199.7 ns | -43.0 ns | The same two revisions | The same three runs |
| Added `hd/1` | 253.3 ns | 195.3 ns | -58.0 ns | The same two revisions | The same three runs |
| Added `map_get/2` on a 1000-key map | 358.9 ns | 295.9 ns | -63.0 ns | The same two revisions | The same three runs |

Source: the verification of [#521](https://github.com/fahchen/pbeam/pull/521) and the section "Step 1" of `docs/interpreter-dispatch-measurements.md`. The first form of the two tables used the default `SipHash` of `HashMap` and measured 351.6 ns to 354.3 ns for each iteration of the local loop, above the 335.0 ns to 340.1 ns of the scan that it replaced; the FNV-1a mixer replaced it, and the same measurement then gave 326.7 ns to 332.8 ns. That pair of numbers alone is not an interleaved before-and-after of two revisions, so this entry states it as the record does and claims no gain from it.

### 2026-10-12 15:12 JST - Dispatch step 0, the baseline, task [#508](https://github.com/fahchen/pbeam/issues/508), pull request [#520](https://github.com/fahchen/pbeam/pull/520)

The step adds the benchmark target, the two probe fixtures, the runner `scripts/dispatch_baseline.sh`, and the mise task. It changes no runtime code, so every number of the step is a baseline value and the before column does not apply.

| Shape | Before | After | Change | Revision | Load or run rule |
| --- | --- | --- | --- | --- | --- |
| Position `call_only`, 200000 iterations | Not applicable | 506.8 ns | The baseline | `078c119` | One benchmark run; each number is the minimum of nine probe runs of one probe; load `{ 5.05 6.37 5.81 }` |
| Local `call_only`, 200000 iterations | Not applicable | 337.5 ns | The baseline | `078c119` | The same run |
| `call_ext_only`, 200000 iterations | Not applicable | 564.0 ns | The baseline | `078c119` | The same run |
| Position `call_only`, 20000 iterations | Not applicable | 504.0 ns | The baseline | `078c119` | The same run |
| Local `call_only`, 20000 iterations | Not applicable | 339.0 ns | The baseline | `078c119` | The same run |
| `call_ext_only`, 20000 iterations | Not applicable | 567.6 ns | The baseline | `078c119` | The same run |
| Added `gc_bif '+'` | Not applicable | 261.9 ns | The baseline added row | `078c119` | The same run; the added numbers move by about 10 percent between runs of the same build |
| Added `tuple_size/1` | Not applicable | 250.1 ns | The baseline added row | `078c119` | The same run |
| Added `element/2` | Not applicable | 250.4 ns | The baseline added row | `078c119` | The same run |
| Added `hd/1` | Not applicable | 265.7 ns | The baseline added row | `078c119` | The same run |
| Added `map_get/2` on a 1000-key map | Not applicable | 399.8 ns | The baseline added row | `078c119` | The same run |
| Terra first frame | Not applicable | 1.36 s | The baseline | `078c119` | `bash scripts/terra_latency.sh` on the release build |
| Terra key latency, four keys | Not applicable | 0.087 s to 0.105 s | The baseline | `078c119` | The same harness; the pinned ERTS run of the same harness gives 0.003 s to 0.008 s |

The pinned ERTS numbers of the same benchmark run are 2.52 ns, 2.52 ns, and 2.50 ns for the position, local, and module-qualified shapes. Source: the verification of [#520](https://github.com/fahchen/pbeam/pull/520) and the section "Step 0" of `docs/interpreter-dispatch-measurements.md`.

## How to add an entry

1. Put the new entry at the top of the `## Entries` section, above every other entry, because the newest entry is first.
2. Name the change in one or two sentences, and name the task issue and the pull request.
3. Give the small table of the affected shapes with the before value, the after value, the change, the revision of each column, and the load or the run rule of each column.
4. Name the source of every number: the section of `docs/interpreter-dispatch-measurements.md`, the merged pull request, or the plan.
5. Use one session that interleaves the two revisions for a claim that the change moved a shape. Do not compare two sessions, and do not present a column boundary of a cumulative table as a measured gain.
6. State **Not landed** in the heading of an entry whose change is not merged, and state the reason.
7. Update the `## Current state` table in the same change when the new entry changes a headline number.
