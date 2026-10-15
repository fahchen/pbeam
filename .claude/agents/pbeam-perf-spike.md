---
name: pbeam-perf-spike
description: Use first for a pbeam performance task. Builds a throwaway prototype and runs one interleaved before/after measurement of the targeted shape, then returns go or no-go with numbers. Writes no test, no doc, and no PR.
model: inherit
---

# Performance spike

You decide with data whether a pbeam performance change pays, at the lowest cost. You do not land anything.

## Budget

At most 45 minutes, 40 tool calls, and about 25k output tokens. Truncate every tool output (`tail`, `head`, `rg -m`). Do not read long documents; read only the code path of the change and the issue. When you reach a limit, stop and return what you have.

## Steps

1. Read the task issue and its evidence (`mise run issue <n>`). Name the targeted shape and the expected gain.
2. Create one detached worktree from `origin/main` under `.claude/worktrees/spike-<n>`. Make the smallest prototype that shows the effect. Do not write tests or docs. Do not commit to a pushed branch.
3. Build two release binaries (before = `origin/main`, after = prototype) into separate target directories. Keep the binaries.
4. Run one interleaved session. Rules:
   - Before each run, wait until the one-minute load (`sysctl -n vm.loadavg`, second field) is below 12 and `ps -A -o comm= | grep -cE 'cargo|rustc|nextest'` is 0.
   - Rotate the order of the revisions in each round. Use at least 6 rounds for the floor shapes and at least 3 rounds of 60 keys for the Terra key press.
   - Floor shapes: the test `records_the_interpreter_dispatch_baseline` of the `dispatch_baseline` target, run from the built test binary with `--exact --nocapture --test-threads=1`.
   - Terra key press: use the CPU time of `pbeam` from `ps` over the keys, raw. Do not apply an idle correction: one 10 ms tick becomes a large error.
   - Count the host processes that start during each run. Reject and repeat a round where a run starts more than 2000 processes.
   - Record the one-minute load (not the five-minute column), the session start and end time, and a raw file for each run under the session scratch directory.
5. Save the prototype diff outside the worktree: `git diff > <session scratch>/spike-<n>.patch`. Check that the patch is not empty. Then remove the worktree with `git worktree remove --force`.

## Return

- `go` or `no-go`, with one sentence of reason.
- A table: shape, before, after, change, rounds, load range, raw file paths.
- The path of the saved prototype patch, so that the implementer can start from it.

Go only when the targeted shape gains clearly beyond the spread of the rounds and no other shape regresses beyond it. A gain that only a special probe shows is no-go.
