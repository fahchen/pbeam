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

### Select the curses library on Linux as ERTS `configure` does

- `src/runtime/host/terminal.rs` links `libtinfo.so.6` by its verbatim name on Linux, as the mise ERTS build links it. A host without this SONAME, for example a musl image, cannot link PBeam. ERTS `configure` uses the first of `tinfo`, `ncurses`, `curses`, `termcap`, and `termlib` that has `tgetent` (`erts/configure.ac:1239-1242`).
- **Trigger:** a supported Linux target without `libtinfo.so.6`, for example the musl build of the specification.
- **Source:** Copilot review of PR #131 (#126).

## Term and loader

### Add float terms

- PBeam has no float term. After #183 (PR #189), the loader accepts `fmove`, `fconv`, `fadd`, `fsub`, `fmul`, and `fdiv`, but each one stops the run with `unsupported BEAM opcode N` when it runs. The only exception is `fconv` of a term that is not a number, which raises `badarith`. `fnegate` is a load error. A float literal stops the run with `unsupported BEAM literal index N` when an instruction reads it. Each BIF that makes or reads a float, such as `erlang:'/'/2`, `erlang:float/1`, and `erlang:float_to_list/1`, stops with `unresolved external MFA`. `is_float` (#167) always fails, because no term is a float. The "Float terms" row of `docs/erts-differences.md` records this.
- The work: a float `Term` variant with the ERTS term order and the `==` arithmetic equality (`1 == 1.0`), float registers and the six instructions with `fnegate`, float literals in the loader, the float BIFs, the ERTS float text in `print_term` (#6), and the ERTS internal hash of a float (`erl_term_hashing.c`).
- **Trigger:** a measured path runs float arithmetic or reads a float literal, for example a `/` operator in a consumer program.
- **Source:** owner question on PR #189 (#183).

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

### Run `error_handler.beam` for an undefined function

- After #166 (PR #180), PBeam gives the result of `error_handler:undefined_function/3` in Rust for a loaded module: it calls `'$handle_undefined_function'/2` or raises `undef` with the first frame. ERTS runs `error_handler.beam` from the export entry (`beam_common.c:1294-1342`). Running the BEAM module needs caller frames in the stack trace (`crash/1` calls `tl/1` on it), `erlang:raise/3` that keeps its frames, and `ensure_loaded/1` through `code_server` or `init` (`error_handler.erl:165-198`). The "Undefined functions" row records the difference.
- **Trigger:** the stack-trace work gives caller frames and `raise/3` frames, or a consumer sets `process_flag(error_handler, Module)`.
- **Source:** review of #166.

### Raise `undef` for an entry module that the run does not load

- After #42 (PR #194), the `pbeam` binary accepts only an argument atom that a loaded module holds. It stops with `argument <M> is not an atom of the loaded modules` or `unresolved external MFA M:F/A`. ERTS raises `undef` through `error_handler` after a load attempt. Doing the same needs a `BeamProgram` API to add an atom, and the undefined-module path of `error_handler` (a code server or a load attempt). The "Run arguments" and "Undefined functions" rows record the difference.
- **Trigger:** a consumer runs an entry module that is not in the loaded BEAM files.
- **Source:** review of #42.

### Halt the PBeam boot with the `init` crash text

- After #6 (PR #195), the `pbeam` binary writes `pbeam: root process exited with <Reason>` for a root exit. Under `erl -noshell -eval`, ERTS writes `Error! Failed to eval: ...` and then `Runtime terminating during boot (<Reason>)` through `init:crash/2` and `erlang:halt/1` (init.erl:665-679, 1552-1567). Doing the same needs `erlang:halt/1,2` with the run end at the halt, `erts_internal:term_to_string/2` (it can use the `print_term` of #6), and an `init` process that watches the boot process.
- **Trigger:** a consumer needs the ERTS crash text byte for byte.
- **Source:** design of #6.

### Keep the frames that `erlang:raise/3` receives

- After #155 (PR #177), `erlang:raise/3` checks its stack trace argument but gives the exception `[]`. ERTS keeps the frames, at most `backtrace_depth` and at least one when the depth is 0. A frame with a location keeps its location, and an old frame without a location (`{Fun, Args}` or `{M, F, A}`) gets the location `[]` (`bif.c:1626-1681`). The "Exception stack trace" row of PR #177 records this.
- **Trigger:** the stack-trace work, or a consumer that reads raised frames.
- **Source:** review of #155.

### Value checks of `process_flag/2` and `system_flag/2`

- After #155 (PR #177), `max_heap_size` (`erl_gc.c:3824-3872`, needs the process minimum heap size), `{monitor_nodes, Options}` (`dist.c:6732-6799`), a `scheduler` value above the scheduler count, and the `system_flag/2` sub-checks of `internal_cpu_topology`, `erts_alloc`, `sequential_tracer`, `schedulers_online`, and `dirty_cpu_schedulers_online` give the runtime error where ERTS raises `badarg`. The "System flags" row of PR #177 records them.
- **Trigger:** a consumer sets one of these flags.
- **Source:** review of #155.

### Check the ordered map iterator crash of ERTS at each OTP pin change

- Pinned ERTS 29.1.1 crashes with SIGSEGV in `erts_internal:map_next/3` when an ordered iterator path holds a key that the map does not hold, or ends with a tail that is not `[]`. It crashes in about one run in four, and in every run with `MallocScribble=1`. The BIF raises `badarg` with a half-built tuple left in its heap fragment, and the GC after the BIF reads the unset element (`erl_map.c:3670-3707`). OTP master has the same code. The "Map iteration" row of `docs/erts-differences.md` records it (PR #177). `tests/logger_start.rs` checks this input on PBeam only (`returns_badarg_for_an_ordered_path_that_crashes_pinned_erts`), and the live `erl` reference run leaves it out. The owner decided not to report it to OTP.
- **Trigger:** a change of the pinned OTP version. Run the reproduction again. If ERTS raises `badarg`, put the input back into the live reference run of `logger_start_probe:ordered/0`, remove the PBeam-only test, and change the row.
- **Source:** investigation of a flaky reference test in the review of #155.

## Tests and tooling

### Load-check every pinned OTP module that loads today

- A test runs `BeamProgram::load` on each pinned OTP 29.1.1 `ebin` module that loads today, so that a decode change that breaks a module fails a test. Today only targeted fixtures guard the loader.
- **Source:** review of PR #121 (#110).

### Move large unit test modules into their own files

- For example `src/runtime/interpreter/tests.rs` with `#[cfg(test)] mod tests;`, with no behavior change.
- **Source:** owner question on PR #119 (`test_atom`).

### Correct the stale "no gc_bif2" fixture comment

- `fixtures/user_drv/user_drv_probe.erl` says PBeam has no `gc_bif2`, but the loader now decodes it.
