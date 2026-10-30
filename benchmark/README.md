# Benchmark of PBeam and the pinned ERTS

This directory holds five scenarios that run the same compiled BEAM modules on the PBeam runtime and on the pinned ERTS. It reports the speed of each scenario as the ratio of the PBeam time to the ERTS time.

The scenarios measure the runtime, not the build and not the boot. The beam files are the same files for the two runs, and the boot time is part of neither number.

## Scenarios

| Scenario | Module | One unit of work | Units of one run |
| --- | --- | --- | --- |
| Integer arithmetic loop | `bench_arith.erl` | One loop iteration: one comparison, four small-integer operations, and one local call | 500000 |
| Spawn and message round trip | `bench_spawn.erl` | One `spawn/3`, one send of the child, and one receive of the parent | 20000 |
| List build and sum | `bench_list.erl` | One list cell: one allocation by the build and one read by the sum | 400000 |
| Map build and lookup | `bench_map.erl` | One key: one insert into the map and one lookup | 100000 |
| Table insert and lookup | `bench_ets.erl` | One object: one insert into an ETS set table and one lookup | 100000 |

Each module exports one function that takes the count as an integer, for example `bench_arith:loop/1`. The count is an argument, so the number of units is in `benchmark.rs` only.

## Method

`tests/integration/benchmark.rs` is an integration test. It compiles `benchmark/*.erl` with the pinned `erlc` into `target/benchmark/beams/`, runs each scenario through the library API of the runtime, and runs the ERTS reference through `erl +S 1 -noshell` with the same beam files on the code path. One scheduler matches the single scheduler thread of the PBeam binary.

- The ERTS run times the call in the guest with `erlang:monotonic_time(nanosecond)`.
- The PBeam run times the call of the runtime from the host with `std::time::Instant`.
- Each number is the minimum of 9 runs.
- The test asserts that the result of each scenario is equal on PBeam and ERTS, so the two runs do the same work.
- The check gate (`mise run check`) runs the same test with 2 runs and one twentieth of the units, so it checks the scenarios on every change and stays short.

## Run

```sh
mise run bench-compare
```

The task runs `bash benchmark/run.sh`. The script prints the revision, the host, the toolchain, the one-minute load, and the table, then runs `cargo test --release --test benchmark -- --nocapture`. The test compiles the Erlang sources with the pinned `erlc` into `target/benchmark/beams/` first.

`mise run bench-compare` needs the Rust release build of the example and the pinned ERTS, and it needs no other setup.

## Results

`mise run bench-compare` prints the table for the host of the run. This directory does not record the numbers of one host as project data, because a number from another host or another session is not comparable. A pull request or an issue that needs a number records it there, with the revision, the host, and the load of the run.

## Limits

- A run-to-run difference of about 10 percent is usual, so compare numbers only from one session on one host.
- The host runs the ERTS JIT. PBeam interprets the same instructions, so the ratio of an instruction-heavy scenario is the cost of the interpreter, not the cost of a missing JIT with an equal instruction cost.
- The scenarios are short and self-contained. They do not measure I/O, binaries, or a process that runs on another scheduler thread.
- The spawn scenario runs one child at a time, so it measures the cost of a process, a send, and a receive, and not the throughput of many live processes.
