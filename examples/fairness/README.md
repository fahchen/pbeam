# Fairness

This example shows that two busy processes on one scheduler both make progress. `Fairness.main/0` of `fairness.ex` spawns the processes `a` and `b`. Each process counts from 1 to 20000 in a loop with no `receive`, and sends its name to the main process at each 10th step. The main process collects the reports. When both processes are done, it prints one line for each run of reports from one process: the name and the first and last step of the run.

The scheduler gives a process a budget of 4000 reductions when the process starts a turn. Each step of the loop costs 1 reduction for the function entry, and each report costs 5 more for the `send`. When a process has used its budget, the scheduler stops it and runs the other process, although the process did not wait. Thus a turn has about 2660 steps, and the reports of `a` and `b` alternate in runs of 266 or 267 reports until each process reaches 20000. A change of one reduction in the budget or in a cost changes the output. The messages are atoms and the loop makes no heap data, so no garbage collection changes the reductions of a process (see "Garbage collection reductions" in [ERTS Differences](../../docs/erts-differences.md)).

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example fairness
```

Add `--erts` to run the same entry on the pinned ERTS. The two commands give the same standard output, no standard error output, and status 0:

```sh
mise run example fairness --erts
```

```text
a 10..2660
b 10..2660
a 2670..5330
b 2670..5330
a 5340..8000
b 5340..8000
a 8010..10660
b 8010..10660
a 10670..13330
b 10670..13330
a 13340..16000
b 13340..16000
a 16010..18660
b 16010..18660
a 18670..20000
b 18670..20000
```

## One scheduler

The ERTS run uses `erl +S 1 -noshell`, which gives ERTS one scheduler. The `pbeam` binary runs all processes on one scheduler thread. With the default number of schedulers, ERTS moves a busy process to a different scheduler, and `a` and `b` run at the same time. Then the lines change from one run to the next, and they do not show the order of the turns on one scheduler. On the pinned ERTS, 30 runs with `+S 1` gave the output above each time, and 10 runs without `+S 1` gave 10 different outputs.
