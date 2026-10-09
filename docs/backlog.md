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

### Add bignum terms

- PBeam stores an integer as a 64-bit signed value. An arithmetic result outside this range, an instruction operand outside it, and `erlang:monotonic_time/1` or `os:system_time/1` with `perf_counter` return a runtime error. ERTS makes a bignum for each integer outside -(2^59) to 2^59 - 1. The "Integer range" row of `docs/erts-differences.md` records this. The entry "Load `SMALL_BIG_EXT` and `LARGE_BIG_EXT` literals that fit in 64 bits" covers only the literals that fit in 64 bits.
- The work: a bignum `Term` variant with the ERTS term order, arithmetic, and internal hash, bignum literals and operands in the loader (`marshal_integer` in `beam_file.c:2132-2214`), and the bignum text in `print_term`.
- **Trigger:** a measured path makes or loads an integer outside the 64-bit range, for example `erlang:monotonic_time(perf_counter)` or `net:shortest_matching_prefix/3`.
- **Source:** "Integer range" row (#110, #154).

### Load local fun literals and run the fun information BIFs

- PBeam loads a `NEW_FUN_EXT` (tag 112) literal as an unsupported literal, so an instruction that reads it returns `unsupported BEAM literal index N`. The pinned compiler makes a local fun with `make_fun3`, not as a literal (measured), so no current path loads one. `erlang:fun_info/2`, `erlang:fun_info_mfa/1`, and `erlang:fun_to_list/1` are `unsupported` BIF table entries (row "Unimplemented BIFs"). `io_lib` calls `erlang:fun_to_list/1` to write a fun (io_lib.erl:699, 826).
- **Trigger:** a measured path loads a local fun literal, calls one of the three BIFs, or formats a fun.
- **Source:** PR #120 (#109).

### Run `erlang:'-'/1` (was #128)

- The measured M2 paths do not run it (pinned ERTS call trace and line counters). #110 removed it. The `is_eq` test of the same task landed in PR #189 (#183), and the `is_float` test is task #167. A negation of a value that is not a literal, such as `-X`, compiles to a `gc_bif` call of `erlang:'-'/1` and returns `unresolved external MFA erlang:-/1` (row "Unimplemented BIFs").
- **Trigger:** a measured consumer negates a value that is not a literal.
- **Source:** review of PR #121 (#110).

## Runtime

### Model the process heap for GC reductions and the message copy cost

- PBeam has no process heap, and its terms keep no sharing and no literal origin. Thus PBeam charges no garbage collection reductions and no message copy cost, and `maps:merge/2` counts the steps of a subtree that both maps share. A change of the `message_queue_data` process flag does not move the messages and sends no adjust signal. The "Garbage collection reductions", "Scheduling", and "Process flags" rows of `docs/erts-differences.md` record these differences, and this entry owns them.
- **Evidence:** a garbage collection costs the process at least 1 reduction, more for a large heap (`erl_gc.c:218-230`). A local send of a message of more than 64 heap words bumps one reduction for each 64 copied words (`erl_message.c:881-886`), and an alias send does the same (`erl_proc_sig_queue.c:2284-2290`). The copy counts each shared subterm once and no literal (`SHCOPY_SEND`). Measured: a send of a list of 10000 integers costs 313 reductions on pinned ERTS and 1 on PBeam. During #233, `lists:reverse/2` and `erlang:'++'/2` copy 60 elements for each reduction only while the heap has free words, and then 40 elements for each reduction to a heap fragment (`erl_bif_lists.c:111-174`, `1144-1192`). Measured: `lists:reverse/2` of 10000 elements costs 166 with a heap of 2000000 words and 2386 with the default heap. `maps:merge/2` takes a subtree that both maps share in memory, and two equal arguments, with no step (`erl_map.c:1258-1263`, `1608-1612`). A change of `message_queue_data` to `off_heap` sends an adjust signal to the process after thread progress (`erl_message.c:1008-1019`, `1022-1135`; `erl_proc_sig_queue.c:3487-3518`).
- The work: a process heap with its size, garbage collection with the ERTS reductions, the heap-fragment fallback of the list BIFs, the shared and literal parts of a term for the send copy and for `maps:merge/2`, and the message place and the adjust signal of `message_queue_data`.
- **Trigger:** a consumer that needs exact interleaving with large messages or with processes that allocate much, or a task that adds a heap-size limit (row "Heap limit").
- **Source:** owner decision on PR #249 (#245).

### Keep the trap state of `maps:from_list/1` and `unicode:characters_to_list/2`

- At each trap, `maps:from_list/1` hashes, sorts, and walks the list again to get its YCF units, and then skips the units that it already used (`src/runtime/interpreter/bif/maps.rs`). `unicode:characters_to_list/2` builds the full list in its first call, and the traps only charge the build (`finish_list` in `src/runtime/interpreter/bif/unicode.rs`). ERTS keeps the YCF state in a magic reference (`utils.c:3733-3819`) and builds the list a part in each call (`erl_unicode.c:1342-1361`). The reductions, the trap points, and the results are the same, so only the host time changes.
- **Evidence:** on a release build, `maps:from_list/1` of 5000, 10000, and 20000 pairs takes 3, 9, and 28 ms, and the tail-recursive build of the same lists takes 1058, 4125, and 16799 ms (measured on the head of PR #249). A list of n pairs traps about n / 16000 times.
- **Trigger:** a measured path converts inputs large enough that the host time of these BIFs matters, for example after a list term change makes the list build fast.
- **Source:** Codex review of PR #249 (#245).

### Follow the endless trap of `maps:from_list/1` at each OTP pin change

- On pinned ERTS, `maps:from_list/1` that finds a bad element or a bad tail after a trap never returns. `BIF_ERROR` in the YCF code returns before YCF clears its trap state, so the trap driver traps again from the last yield without end (`utils.c:3757-3773`). PBeam returns the runtime error `unsupported maps:from_list/1 call: maps:from_list/1 finds a bad element after a trap, and ERTS then traps again forever` (`src/runtime/interpreter/bif/maps.rs`). The "Scheduling" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Evidence:** on `erl +S 1`, `maps:from_list/1` of 170000 pairs and then the atom `bad` did not return in 20 seconds. The same call with 100 pairs and `bad` raises `badarg`, because it does not trap.
- **Option:** report the endless trap to OTP, and follow the report.
- **Trigger:** a change of the pinned OTP version. Run the probe again on the new ERTS. If the call returns, give the same result in PBeam and change the row.
- **Source:** #245 (PR #249) and #258.

### Suspend `persistent_term:put/2` as ERTS does

- `persistent_term:put/2` of a new value switches out the caller in PBeam, and the new value is visible at once. ERTS makes the new table visible after thread progress, and suspends the caller until then (`erl_bif_persistent.c:774-779`). With `erl +S 1`, other processes then run 2 to 5 turns before the caller continues.
- While `persistent_term:get/0` traps, ERTS keeps the update permission until its last chunk, and `persistent_term:put/2` of another process suspends until ERTS releases the permission, and then tries again (`erl_bif_persistent.c:333-345`, `692-694`, `1311-1335`, `1353-1357`). PBeam changes the table at once, and the trapped call returns the table of its first call.
- The "Persistent terms" row of `docs/erts-differences.md` records the two suspensions, and this entry owns them. Thread progress depends on all managed threads of the node, so PBeam on one scheduler does not model it.
- **Trigger:** multiple schedulers (M10, #27), or a consumer that needs the exact interleaving around `persistent_term:put/2`, also a put while `persistent_term:get/0` traps.
- **Source:** #234 (PR #243), #245 (PR #249), and #258.

### Complete a trapped `ets:insert/2` in the process that uses the table

- PBeam puts all objects of a list insert into the table before the insert traps after its copy. Another process that then uses the table sees all objects with no cost, and the insert still uses its units. On ERTS, the table is busy while the insert traps, and the first process that uses it completes the insert with its own reductions. The inserting process then returns `true` with no cost (`handle_lacking_permission`, `erl_db.c:769-798`; `erl_db.c:2223-2244`). The objects are the same, and only the reductions and the turns are different. The "ETS tables" row of `docs/erts-differences.md` records this, and this entry owns it.
- `ets:delete/1` takes the busy table in the same way, so it completes the insert first, and the insert then returns `true` (`erl_db.c:2232-2244`, `3021-3029`).
- **Trigger:** a consumer uses a table while a long list insert traps, or the task that adds `ets:delete/1`.
- **Source:** #234 (PR #243).

### Read a negative reduction budget in each BIF as ERTS does

- A hashmap call of `erts_internal:map_next/3` subtracts 4 for each pair with no lower limit, so the budget can be negative (`erl_map.c:4126`). Each ERTS BIF reads a negative budget in its own way: `lists:member/2` traps at once, and `erlang:length/1` and `lists:reverse/2` read it as a large unsigned count (`erl_bif_lists.c:1114-1130`, `1207`; `erl_bif_guard.c:280`). When a BIF reads a negative budget, through `apply` or as a guard BIF before the next call or return, PBeam stops the run with `unsupported negative reduction budget in a BIF after erts_internal:map_next/3` (`bif_reductions_left` in `src/runtime/process.rs`). `maps.erl` calls `erts_internal:map_next/3` only before a call or a return (`maps.erl:525-542`, `1104-1111`), so the `maps` functions do not get this error. The "Map iteration" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Evidence:** `logger_start_probe:negative_budget/0` makes the budget negative before `length/1`. ERTS returns `ok`, and PBeam returns the runtime error (`tests/integration/logger_start.rs`).
- **Trigger:** a measured path calls `erts_internal:map_next/3` directly and then a BIF that reads the budget.
- **Source:** #234 (PR #243).

### Run `integer_to_list/1` and `integer_to_binary/1` of a large integer with no reductions left

- When the digits of a bignum divided by 16 are more than the reductions left, ERTS runs `integer_to_list/1` or `integer_to_binary/1` again on a dirty CPU scheduler (`bif.c:3283-3299`; `binary.c:92-108`). PBeam has no dirty scheduler. For an integer from 2^59 to 2^63 - 1 with no reductions left, it returns the runtime error `unsupported erlang:integer_to_list/1 of N with 0 reductions left: ERTS runs it on a dirty CPU scheduler`, or the same error for `erlang:integer_to_binary/1`. The "Scheduling" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Trigger:** a measured path converts such an integer at the end of a budget, or a task adds dirty schedulers.
- **Source:** #234 (PR #243), #268.

### Read the clock after `ERTS_CHECK_TIME_REDS` reductions

- PBeam reads the run clock and expires each due timer before it takes the next process. An ERTS scheduler reads the clock only when its processes used `ERTS_CHECK_TIME_REDS` (`CONTEXT_REDS`, 4000) reductions after the last read, and before it sleeps. Between two reads, it expires the timers on the last time that it read (`erl_process.c:59`, `9797-9810`; `erl_process.h:528-535`; `erl_time_sup.c:280`). Thus on ERTS a due timer can expire some turns later than on PBeam. The "Timers" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Trigger:** a measured path depends on the turn in which a timer expires while other processes run.
- **Source:** #229 (PR #236).

### Run the other binary-match instructions and `bs_match` commands

- PBeam runs `bs_start_match3`, `bs_get_position`, `bs_set_position`, `bs_get_utf8`, and `bs_match` with the commands `ensure_at_least` and `ensure_exactly`. The other `bs_match` commands (`binary`, `integer`, `skip`, `get_tail`, and `=:=`) stop the run with `unsupported bs_match command <Name>`. `bs_start_match3` that copies a match context to another register stops the run with `unsupported bs_start_match3 that copies the match context in <Source> to <Destination>`. Each other binary-match instruction, such as `bs_start_match4`, `bs_get_tail`, `bs_skip_utf8`, and `bs_get_binary2`, stops the run with `unsupported BEAM opcode N`. The "Binary matching" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Evidence:** `binary_match_probe:byte(<<7,8>>)` stops with `unsupported bs_match command integer`.
- **Trigger:** a measured path matches a binary with one of these commands or instructions. Add only the commands and instructions that the path needs.
- **Source:** #219 (PR #227).

### Restore the terminal mode of fd 0 at runtime exit (was #136)

- PBeam restores fd 0 when the owner of a TTY resource exits, but not at runtime exit as ERTS does (`erl_child_setup.c:563-576`, `sys.c:182-190`). The "Terminal mode at exit" row of `docs/erts-differences.md` records it.
- **Trigger:** a PBeam program uses an input mode that leaves fd 0 raw.
- **Source:** review of PR #133 (#115).

### Write the output in a locale whose codeset is not UTF-8

- In such a locale, `prim_tty` writes in latin1 mode. `prim_tty:to_latin1/2` calls `is_usascii/1` (prim_tty.erl:1309-1321), which PBeam runs as ERTS does. Thus PBeam writes ASCII output as ERTS does. Output that is not ASCII calls `binary_to_latin1/2`, which builds each character with `char_to_latin1/2` (prim_tty.erl:1323-1330). #269 runs the `string` and `binary` segments of `bs_create_bin`. `char_to_latin1/2` also needs the `integer` segment of `bs_create_bin` (prim_tty.erl:1329-1330) and `erlang:integer_to_binary/2` (prim_tty.erl:1325-1328). PBeam stops with `unsupported bs_create_bin segment integer` and status 1 (measured with `IO.puts/1` of the UTF-8 binary of `é`). ERTS writes the Latin-1 output with status 0. The "Boot" row records this.
- The work: the `integer` segment of `bs_create_bin` and `erlang:integer_to_binary/2` for `char_to_latin1/2`.
- **Trigger:** a supported run that writes output that is not ASCII in a locale whose codeset is not UTF-8, for example a macOS shell with `LANG`, `LC_ALL`, and `LC_CTYPE` unset.
- **Source:** "Boot" row (#139, #183, #219, #269).

### Run the unsupported BIFs that OTP code calls

- The "Unimplemented BIFs" row lists the `unsupported` BIF table entries that OTP code often calls. Each returns `unresolved external MFA M:F/A`. The groups:
  - Timers: `erlang:start_timer/4`, `erlang:send_after/3,4`, `erlang:cancel_timer/1`, and `erlang:read_timer/1,2`. `erlang:start_timer/3` and `erlang:cancel_timer/2` already use the timer list of #150 (PR #172).
  - Aliases: `erlang:alias/1`, which `erlang:alias/0` calls, and `erlang:unalias/1`.
  - Equality: `erlang:'=='/2` and `erlang:'/='/2`. They give other results than `=:=` and `=/=` only for floats (see "Add float terms").
  - Persistent terms: `persistent_term:erase/1`, `persistent_term:put_new/2`, and `persistent_term:info/0`.
  - Signals: `erlang:exit/3`.
  - Maps: `maps:get/2` and `maps:is_key/2` through a fun call. The compiler changes a direct call to the `map_get` and `is_map_key` BIFs, which PBeam runs.
- **Trigger:** a measured path calls one of them. Add each group in its own task.
- **Source:** audit of the BIF table at the M2 close.

### Run the remaining test instructions for `io:format/2`

- The loader accepts `is_ne`, `is_number`, `is_port`, `is_bitstr`, and `is_any_native_record`, but each one stops the run with `unsupported BEAM opcode N`. Under the PBeam boot, `io:format/2` with `~w` of a list, a tuple, or a map stops at `is_port` (io_lib.erl:820). `~p` of an integer stops at `is_bitstr`, and `~p` of a tuple stops at `is_any_native_record` (measured). The "Unimplemented instructions" row records these instructions and `bs_init_writable`, `gc_bif3`, `bs_create_bin`, `get_record_elements`, and `put_record`. A format without a term argument, `~w` of an integer, and `~s` of a string already work. PR #188 also named `net_kernel:dflag_unicode_io/1` and `erl_scan:reserved_word/1`. The first is a BIF that PBeam runs (PR #88), and the second is a function of `erl_scan.beam`, which a run gives with the other OTP modules.
- **Trigger:** a consumer or a probe prints a compound term with `io:format/2`, for example to compare its output byte for byte through the `pbeam` binary.
- **Source:** PR #188 (#179) and measured probes at the M2 close.

### Turn on native bindings only after `erlang:load_nif/2`

- The native bindings of `prim_tty` are active from module load, also before `erlang:load_nif/2` and after a failed load. ERTS replaces a stub body only after a successful load (`erl_nif.c:4990-5007`). The "NIF stubs" row records this. PR #132 proposed the task "[m2-e1] Turn on native bindings only after erlang:load_nif/2", but it was not filed.
- **Trigger:** a path calls a `prim_tty` NIF before `erlang:load_nif/2` or after a failed load, or a second module gets native bindings.
- **Source:** PR #132 (#127).

### Poll fd 0 while processes run

- PBeam waits for the read select of fd 0 only when no process can run (#299). ERTS also waits for it in a poll thread while processes run (erl_process.c:3339-3372), so input arrives while other processes run. Thus in PBeam, a run in which a process can always run never gets the `{select, TTY, Ref, ready_input}` message. The "Input select" row of `docs/erts-differences.md` records this.
- The work: a poll of fd 0 with no timeout at a scheduler switch point, for example after a number of reductions, as an ERTS scheduler checks its own poll set (erl_process.c:9946-9960).
- **Trigger:** a program reads standard input while one of its processes can always run, for example a busy worker.
- **Source:** PR for #299.

### Deliver host signals to `erl_signal_server`

- For `handle`, `os:set_signal/2` installs a handler that does nothing, so `erl_signal_server` never gets a `{notify, Signal}` message. PBeam also sets no signal disposition at the start of a run. ERTS sends `{notify, Signal}` from its signal dispatcher thread (sys.c:589-607, 1106-1180). At emulator start, it installs handlers for `SIGINT`, `SIGQUIT`, `SIGTERM`, and `SIGUSR1`, and ignores `SIGCHLD` (sys.c:799-823). The "OS signals" row records this.
- The work: an event source in the scheduler for host signals, and the start-time dispositions.
- **Trigger:** a program handles a host signal, or a supported run must act on `SIGTERM` or `SIGINT` as ERTS does.
- **Source:** PR #133 (#115).

### Run `at_exit` callbacks and flush the logger before the root exit

- The PBeam boot does not run `at_exit` callbacks and does not flush the logger before the root process returns. Thus a log event that is still in the queue of the async default handler at the root exit is lost. `elixir -e` runs the callbacks, then `Logger.flush()`, and then `System.halt/1` (kernel/cli.ex:58-71). The "Run end" row records this. `fixtures/io_fail` calls `logger_std_h:filesync(default)` so that its report is written before the root exit.
- **Trigger:** a consumer logs an event just before it returns and needs the event, or a program registers `at_exit` callbacks.
- **Source:** PR #182 (#169).

### Compose decomposed UTF-8 native names on macOS

- On macOS, `os:getenv/1` of a value that is valid UTF-8 and has one of the 61 ERTS composition candidates returns the runtime error `unsupported composition of the native name code point N on macOS`. ERTS composes the code points with `compose_tab` (`erl_unicode.c:1418-1428`, `1569-1630`; `erl_unicode_normalize.h:1523-1690`). This composition is not Unicode NFC. The "OS environment" row records this.
- The work: the 1040 entries of `compose_tab` and the ERTS composition algorithm.
- **Trigger:** a supported macOS path reads a decomposed native name, for example an environment value.
- **Source:** PR #191 (#185).

### Set the start variables and `init` flags of the `erl` script

- PBeam has no `erl` script and no `erlexec`. Thus `os:getenv/1` gives the host value or `false` for `EMU`, `ROOTDIR`, `BINDIR`, and `PROGNAME`, and `PATH` has no `BINDIR` prefix. The `init` request process of `pbeam_boot` has only the flag `noshell`. The `erl` script exports the four variables (`erl.src.src:52-58`), `erlexec` puts `BINDIR` and `ROOTDIR/bin` at the front of `PATH` (`erlexec.c:553-621`), and the flags of `init` also have `root`, `bindir`, `progname`, and `home`. The "OS environment" and "Boot" rows record this.
- **Trigger:** a consumer reads one of these variables or flags, for example `init:get_argument(root)`.
- **Source:** PR #191 (#185).

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

### Keep the run alive after a root `exit(normal)` as ERTS does

- When the root process calls `exit(normal)`, the `pbeam` binary writes `pbeam: root process exited with normal` and exits with status 1. On ERTS, `start_it/1` writes `Error! Failed to eval: <expression>` and raises the exit again (init.erl:1552-1567). The boot process exits `normal`, `init` goes to its main loop (init.erl:700-704), and the run does not end. The "Run end" row of `docs/erts-differences.md` records this.
- **Evidence:** a module `haltp` with `exit_normal() -> exit(normal).`. On pinned ERTS, `timeout 10 erl -noshell -pa <dir> -eval 'haltp:exit_normal()'` writes `Error! Failed to eval: haltp:exit_normal()`, then `SIGTERM received - shutting down` at the timeout, and exits with status 124. On the `pbeam` binary, `pbeam pbeam_boot start haltp exit_normal`, with the BEAM files of `examples/hello_world/README.md`, writes `pbeam: root process exited with normal` and exits with status 1.
- The work: an `init` process that links to the boot process and keeps the run alive after a `normal` exit, as "Halt the PBeam boot with the `init` crash text" needs for the other exits.
- **Trigger:** a consumer needs the ERTS result of a root `exit(normal)`, for example a program that ends its root process and keeps its other processes alive.
- **Source:** M2 exit check (#220).

## Exceptions

### `erlang:error/2` and the `throw` class

- After #155 (PR #177), `erlang:error/2` is still `unsupported`; `re:ucompile/2` (`re.erl:1742-1751`) stops at it. `erlang:throw/1` and `erlang:raise(throw, R, Stack)` give runtime errors.
- **Trigger:** a measured path raises through them.
- **Source:** review of #155.

### Run `error_handler.beam` for an undefined function

- After #166 (PR #180), PBeam gives the result of `error_handler:undefined_function/3` in Rust for a loaded module: it calls `'$handle_undefined_function'/2` or raises `undef` with the first frame. ERTS runs `error_handler.beam` from the export entry (`beam_common.c:1294-1342`). Running the BEAM module needs caller frames in the stack trace (`crash/1` calls `tl/1` on it), `erlang:raise/3` that keeps its frames, and `ensure_loaded/1` through `code_server` or `init` (`error_handler.erl:165-198`). The "Undefined functions" row records the difference. The `error_handler` code also costs reductions: on `erl +S 1`, a call that goes to `'$handle_undefined_function'/2` costs 11 and a caught `undef` costs 13. PBeam charges only the entry and the `return` of the handler, and nothing for an `undef` (owner decision in #7).
- **Trigger:** the stack-trace work gives caller frames and `raise/3` frames, or a consumer sets `process_flag(error_handler, Module)`.
- **Source:** review of #166.

### Raise `undef` for an entry module that the run does not load

- After #42 (PR #194), the `pbeam` binary accepts only an argument atom that a loaded module holds. It stops with `argument <M> is not an atom of the loaded modules` or `unresolved external MFA M:F/A`. ERTS raises `undef` through `error_handler` after a load attempt. Doing the same needs a `BeamProgram` API to add an atom, and the undefined-module path of `error_handler` (a code server or a load attempt). The "Run arguments" and "Undefined functions" rows record the difference.
- **Trigger:** a consumer runs an entry module that is not in the loaded BEAM files.
- **Source:** review of #42.

### Halt the PBeam boot with the `init` crash text

- After #6 (PR #195), the `pbeam` binary writes `pbeam: root process exited with <Reason>` for a root exit. Under `erl -noshell -eval`, ERTS writes `Error! Failed to eval: ...` and then `Runtime terminating during boot (<Reason>)` through `init:crash/2` and `erlang:halt/1` (init.erl:665-679, 1552-1567). Doing the same needs `erlang:halt/1` with a string, which writes the slogan and a crash dump (`erlang:halt/2` with an integer status runs since #301), `erts_internal:term_to_string/2` (it can use the `print_term` of #6), and an `init` process that watches the boot process.
- **Trigger:** a consumer needs the ERTS crash text byte for byte.
- **Source:** design of #6.

### Send the uncaught-exception report to `logger`

- PBeam writes no report for an uncaught exception. For an uncaught `error` or `throw`, ERTS sends `{log, error, "Error in process ~p with exit value:~n~p~n", Args, Metadata}` to the registered `logger` process (beam_common.c:697-718, erl_trace.c:2812-2820). The "Error report" row records this. `pbeam_boot` starts `logger` (PR #188), so the receiver exists. PR #188 said that #73 tracks this work, but #73 is closed.
- **Trigger:** a consumer needs the report of a process that crashes.
- **Source:** PR #188 (#179).

### Keep the frames that `erlang:raise/3` receives

- After #155 (PR #177), `erlang:raise/3` checks its stack trace argument but gives the exception `[]`. ERTS keeps the frames, at most `backtrace_depth` and at least one when the depth is 0. A frame with a location keeps its location, and an old frame without a location (`{Fun, Args}` or `{M, F, A}`) gets the location `[]` (`bif.c:1626-1681`). The "Exception stack trace" row of PR #177 records this.
- **Trigger:** the stack-trace work, or a consumer that reads raised frames.
- **Source:** review of #155.

### Value checks of `process_flag/2` and `system_flag/2`

- After #155 (PR #177), `max_heap_size` (`erl_gc.c:3824-3872`, needs the process minimum heap size), `{monitor_nodes, Options}` (`dist.c:6732-6799`), a `scheduler` value above the scheduler count, and the `system_flag/2` sub-checks of `internal_cpu_topology`, `erts_alloc`, `sequential_tracer`, `schedulers_online`, and `dirty_cpu_schedulers_online` give the runtime error where ERTS raises `badarg`. The "System flags" row of PR #177 records them.
- **Trigger:** a consumer sets one of these flags.
- **Source:** review of #155.

### Check the ordered map iterator crash of ERTS at each OTP pin change

- Pinned ERTS 29.1.1 crashes with SIGSEGV in `erts_internal:map_next/3` when an ordered iterator path holds a key that the map does not hold, or ends with a tail that is not `[]`. It crashes in about one run in four, and in every run with `MallocScribble=1`. The BIF raises `badarg` with a half-built tuple left in its heap fragment, and the GC after the BIF reads the unset element (`erl_map.c:3670-3707`). OTP master has the same code. The "Map iteration" row of `docs/erts-differences.md` records it (PR #177). `tests/integration/logger_start.rs` checks this input on PBeam only (`returns_badarg_for_an_ordered_path_that_crashes_pinned_erts`), and the live `erl` reference run leaves it out. The owner decided not to report it to OTP.
- **Trigger:** a change of the pinned OTP version. Run the reproduction again. If ERTS raises `badarg`, put the input back into the live reference run of `logger_start_probe:ordered/0`, remove the PBeam-only test, and change the row.
- **Source:** investigation of a flaky reference test in the review of #155.

## Tests and tooling

### Load-check every pinned OTP module that loads today

- A test runs `BeamProgram::load` on each pinned OTP 29.1.1 `ebin` module that loads today, so that a decode change that breaks a module fails a test. Today only targeted fixtures guard the loader.
- **Source:** review of PR #121 (#110).

### Correct the stale "no gc_bif2" fixture comment

- `fixtures/user_drv/user_drv_probe.erl` says PBeam has no `gc_bif2`, but the loader now decodes it.

### Use `INSTRUCTION_CALLS` in `erlang:function_exported/3`

- `function_exported/3` in `src/runtime/interpreter/bif/erlang.rs` repeats the entries of `INSTRUCTION_CALLS` (`src/runtime/interpreter/bif.rs`) in a `matches!`. The two lists hold the same entries, so it can call `INSTRUCTION_CALLS.contains`.
- **Trigger:** the next change to `INSTRUCTION_CALLS` or `function_exported/3`.
- **Source:** PR #173 (#151).

### Set `TERM` and `LC_ALL` of the test commands with one helper

- `tests/feature/boot.rs`, `tests/integration/time.rs`, `tests/integration/user_drv.rs`, and `tests/integration/prim_tty.rs` set `TERM=dumb` and `LC_ALL=C.UTF-8` on the command, as `run_dumb_utf8` of `tests/common/run.rs` does. They do not use a time limit, and `tests/integration/prim_tty.rs` reads only the exit status with its own stdin, stdout, and stderr. Thus each one can call `run_dumb_utf8`, or a helper that only sets the two variables.
- **Trigger:** the next change to one of these test runners.
- **Source:** PR #192 (#186).

### Cache the Rust build in CI

- PR #294 (#292) measured a cache of `target/` with `CARGO_INCREMENTAL=1`. Without a cache, the test build of main run 37929970799 took 68s. With the default `cargo llvm-cov` clean, which removes the incremental state of `pbeam-runtime`, warm builds took 60s and 75s, so the cache gave no gain. A dependency-only cache gives the same result. With `--no-clean`, warm builds took 19s to 30s, but the restore took 23s to 26s and the save 8s to 13s. Against the 68s main baseline, the net change was from 1s slower to 18s faster per run, which is smaller than the run-to-run variation of the test time (244s to 412s). Each entry is about 600 MB, and the repository cache limit is 10 GB.
- `--no-clean` needs a removal of old `*.profraw` files and of old test executables before each coverage run; without it, the coverage gate reported 9533/10116 after one profile change. The gain does not justify this risk to the merge gate.
- **Trigger:** the test build becomes much longer than the cache transfer (for example, more crates or a larger dependency tree), or `cargo llvm-cov` keeps the incremental state by itself.
- **Source:** PR #294 (#292).
