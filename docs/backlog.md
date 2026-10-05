# Backlog

This file lists optional work and ideas to consider later. Each item has no current consumer or no owner decision yet, so it is not a GitHub issue. Scheduled work is in GitHub issues and on the [project board](https://github.com/users/fahchen/projects/2).

When an item gets a consumer or an owner decision, open a task issue for it, link the issue here, and remove the item in the PR that closes the issue.

Each entry gives the reason, the evidence, and the trigger that makes it a task.

## Build and dependencies

### Use the `erlang/pcre2` fork as a git submodule for PCRE2

- **Now:** `vendor/pcre2/` holds 29 files that are byte-identical to `OTP-29.1.1:erts/emulator/pcre/` (same git blob hashes). `build.rs` compiles 19 of them with `-DERLANG_INTEGRATION`.
- **Option:** a submodule of `erlang/pcre2`, branch `ERLANG_INTEGRATION`, pinned to commit `58012191`, gives the same source. `build.rs` must then generate `pcre2.h` from `pcre2.h.in` (four substitutions) and copy `pcre2_chartables.c.dist`. CI needs `submodules: true`, and each agent worktree needs `git submodule update --init`.
- **Not other options:** upstream PCRE2 10.47 lacks the Erlang yield patch and gives other `export` bytes; an `erlang/otp` submodule fetches about 640 MB; the `pcre2-sys` crate bundles 10.46 with JIT.
- **Risk:** the pin is an untagged commit on a moving branch (its head is already PCRE2 10.48).
- **Trigger:** the next OTP version update, or a decision to keep no third-party source in the tree.
- **Source:** owner review of PR #124.

## Term and loader

### Load `SMALL_BIG_EXT` and `LARGE_BIG_EXT` literals that fit in 64 bits

- ERTS `dec_term` makes a small integer when the value fits (`external.c:4457-4495`). PBeam loads `SMALL_INTEGER_EXT` and `INTEGER_EXT` only, so a fixture keeps such integers out of literals with `id/1`.
- **Trigger:** a measured path loads such a literal.
- **Source:** review of PR #160 (#138).

### Run `erlang:'-'/1` and `is_eq` (was #128)

- The measured M2 start path does not run them (pinned ERTS call trace and line counters). #110 removed them. The `is_float` test of the same task now has a consumer on the logger start and is task #167.
- **Trigger:** a measured consumer, for example a `~e`, `~f`, or `~g` format, or `lists:usort/1` with three elements.
- **Source:** review of PR #121 (#110).

## Runtime

### Restore the terminal mode of fd 0 at runtime exit (was #136)

- PBeam restores fd 0 when the owner of a TTY resource exits, but not at runtime exit as ERTS does (`erl_child_setup.c:563-576`, `sys.c:182-190`). The "Terminal mode at exit" row of `docs/erts-differences.md` records it.
- **Trigger:** a PBeam program uses an input mode that leaves fd 0 raw.
- **Source:** review of PR #133 (#115).

### Supervise the boot IO servers as `kernel_sup` does

- `kernel_sup` runs the IO servers with `one_for_all` and intensity 0, so the node halts with `Kernel pid terminated` when one exits. The PBeam boot does not supervise them.
- **Trigger:** an IO server can exit on a supported path, or an acceptance check needs the halt.
- **Source:** review of PR #161 (#139).

### Give the boot servers an `application_master` group leader

- Add `erlang:process_info/2` with the `group_leader` item, so that the group leader difference of the boot can be tested.
- **Trigger:** a path reads the group leader of a boot server.
- **Source:** review of PR #161 (#139).

### Model the ERTS timer wheels

- After #150 (PR #172), PBeam keeps one timer list ordered by end time. ERTS uses soon and later wheels and a high-level timer tree, so a timer of 16384 ms or more that moves into a soon slot expires after the timers with the same end that are already in the slot (pinned probe gives `[b,a]`).
- **Trigger:** a measured path depends on the order of timers with the same end.
- **Source:** review of #150.

### Accept every `erlang:spawn_opt/4` option

- PBeam accepts only `link` and `monitor` and gives a runtime error for each other option. ERTS accepts many more and raises `badarg` for an option it does not know (`erts_parse_spawn_opts`). The "Spawn options" row records this.
- **Trigger:** a consumer needs another spawn option.
- **Source:** review of #152.

### Parse BIF option lists with one shared function

- Each BIF that takes an option list has its own parser: `timer_options` for `erlang:cancel_timer/2` (PR #172), `parse_options` for `re:compile/2`, and the `spawn_opt/4` options. Extract one function that takes the option term and a schema of the accepted options, and returns the parsed values or `badarg` for an option list that is not valid. Each BIF then gives only its schema.
- **Trigger:** the next BIF that takes an option list, or a change to an existing option parser.
- **Source:** owner review of PR #172 (#150).

### Delete one key of a hashmap in place

- `maps:remove/2` (#129) rebuilds the map, which is O(n). ERTS changes one HAMT path.
- **Trigger:** a hot path removes keys from large maps.

### Transfer an ETS table to a live heir

- `give_away_to_heir`, the `ETS-TRANSFER` message, and the heir checks of `ets:new/2`.
- **Trigger:** the first consumer of `ets:give_away/3`, `ets:setopts/2`, or a live heir.
- **Source:** #112.

## Exceptions

### `erlang:error/2` and the `throw` class

- After #155 (PR #177), `erlang:error/2` is still `unsupported`; `re:ucompile/2` (`re.erl:1742-1751`) stops at it. `erlang:throw/1` and `erlang:raise(throw, R, Stack)` give runtime errors.
- **Trigger:** a measured path raises through them.
- **Source:** review of #155.

### Keep the frames that `erlang:raise/3` receives

- After #155 (PR #177), `erlang:raise/3` checks its stack trace argument but gives the exception `[]`. ERTS keeps the frames (at most `backtrace_depth`) with the location `[]`. The "Exception stack trace" row of PR #177 records this.
- **Trigger:** the stack-trace work, or a consumer that reads raised frames.
- **Source:** review of #155.

### Value checks of `process_flag/2` and `system_flag/2`

- After #155 (PR #177), `max_heap_size` (`erl_gc.c:3824-3872`, needs the process minimum heap size), `{monitor_nodes, Options}` (`dist.c:6732-6799`), a `scheduler` value above the scheduler count, and the `system_flag/2` sub-checks of `internal_cpu_topology`, `erts_alloc`, `sequential_tracer`, `schedulers_online`, and `dirty_cpu_schedulers_online` give the runtime error where ERTS raises `badarg`. The "System flags" row of PR #177 records them.
- **Trigger:** a consumer sets one of these flags.
- **Source:** review of #155.

## Tests and tooling

### Load-check every pinned OTP module that loads today

- A test runs `BeamProgram::load` on each pinned OTP 29.1.1 `ebin` module that loads today, so that a decode change that breaks a module fails a test. Today only targeted fixtures guard the loader.
- **Source:** review of PR #121 (#110).

### Move large unit test modules into their own files

- For example `src/runtime/interpreter/tests.rs` with `#[cfg(test)] mod tests;`, with no behavior change.
- **Source:** owner question on PR #119 (`test_atom`).

### Correct the stale "no gc_bif2" fixture comment

- `fixtures/user_drv/user_drv_probe.erl` says PBeam has no `gc_bif2`, but the loader now decodes it.
