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

### Add bitstring terms

- PBeam has no bitstring term whose size is not a multiple of 8 bits, so `is_bitstr` passes only for a binary. The loader keeps a `BIT_BINARY_EXT` literal as an unsupported literal, so `byte_size(<<1:3>>)` stops with `unsupported BEAM literal index N`. ERTS gives `1` (`erl_bif_guard.c:376-393`). The "Bitstring terms" row records this.
- The work: a bitstring `Term` variant with the ERTS term order, `BIT_BINARY_EXT` literals in the loader, the bit count in `erlang:byte_size/1`, and the bitstring forms of the binary instructions and BIFs, such as the `binary` and `get_tail` commands of `bs_match`, `bs_get_binary2`, and `bs_get_tail` of a size that is not a multiple of 8 bits.
- **Trigger:** a measured path makes or loads a bitstring that is not a binary.
- **Source:** #359 and #360.

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

### Cache the decoded modules of a run

- Each run decodes the BEAM bytes of its start set, and each module that loads on demand decodes its bytes again at the load (`erts_internal:prepare_loading/2`, `src/beam/code.rs`). A run that loads the same module more than one time, for example a second run of an example in one OS process, decodes it again.
- **Trigger:** the start time of Terra Counter, or of another example, is slower than on `main` after the start set of #448. #448 gives the PBeam run the start set of the build and loads each other module on demand, so it decodes fewer modules at the start and each on-demand module at its first call.
- **Source:** #448, `docs/code-loading.md` section 8 (Q9).

### Build the large tuple tree for a retained-version consumer

- **Reason:** no consumer in this repository updates a tuple of more than 32 slots while an old version stays reachable.
- **Evidence:** the only tuple write path of this runtime is `update_record` (`src/runtime/interpreter.rs:1363`), which copies the whole array and discards the hint of the instruction (`src/beam/code/instruction.rs:731-743`). The only live compiled consumer is the pinned `gen_statem.beam` with `{update_record,{atom,inplace},6,...}` in `loop_t0q/5` and `{atom,copy},7` in `enter/8`. `fixtures/user_drv/user_drv_state_check.erl:16` updates a tuple of 19 slots through `call_ext erlang:setelement/3`, that BIF is unsupported (`src/runtime/interpreter/bif/table.rs:299`), and the test runs on ERTS only (`tests/integration/user_drv.rs:270`). A retained old version appears only in a hand-built 4-tuple at `tests/runtime/interpreter.rs:1362`.
- **Trigger:** a measured consumer that updates a tuple of more than 32 slots and keeps the old version reachable.
- **Source:** the consumer audit of the term proposal, and issue #556.

### Build bounded map collision storage for a collision-write consumer

- **Reason:** the collisions of a map in this repository reach a count of 2, no measured consumer writes a colliding map while it retains the old root, and every retained-root map write of the measured paths is collision-free.
- **Evidence:** `fixtures/logger_start/logger_start_probe.erl:224-226` and `tests/term/map.rs:666` read collision pairs; the largest count is 2 (`tests/term/map.rs:629-631`, `:667-672`). Collision writes do exist in unit tests: `tests/term/map.rs:670-675` inserts two colliding keys, replaces one of them, and changes the other through `get_mut`, and `:842-846` writes a collision pair. None of them is measured, none is in `scripts/term_costs.sh`, and none holds a retained old root while it writes. Every retained-root write of the measured paths is collision-free (`fixtures/map/map_share_probe.erl:7-19`, `tests/term/map.rs:404-432`, `:455-479`, `:858-874`).
- **Trigger:** a measured write path with a retained root and a collision count above 16.
- **Source:** the consumer audit of the term proposal, and issue #557.

### Combine the measured instruction pairs

- **Reason:** row 10 of the term proposal combines the measured instruction pairs, and that form was measured and rejected in the dispatch work. The row has no consumer until a combined form measures a gain.
- **Evidence:** task #517 of the dispatch epic built the combined pairs for `test is_eq_exact` with `gc_bif '-'`. The gate measures the local `call_only` floor loop at 200000 iterations: the combined form measures 165.7 ns for each iteration against 163.2 ns of the two separate instructions, and the position shape measures 165.4 ns against 163.5 ns. The added rows of one instruction are 9.1 ns to 12.1 ns slower. The task closed with no landed change, and issue #517 and the closed draft PR #538 hold the measurement. The plan of record is `docs/interpreter-dispatch-plan.md`.
- **Trigger:** a combined form that measures a gain of at least 10 percent on the local floor loop against the two separate instructions, with the session control of its own revision.
- **Source:** task #517 and the step 9 measurement of `docs/interpreter-dispatch-measurements.md`.

### Pin the select_val linear bound between 17 and 32 choices

- **Reason:** `SELECT_VALUE_LINEAR_LIMIT` is 32, and it is a measured estimate. The end-to-end session of task #545 measures a loss of the ordered form at 16 choices and a win of 458.005 ns at 256 choices, and no end-to-end probe covers 17 to 32 choices, so the crossover of the average case is unknown between those points.
- **Evidence:** the session of task #545 in `tests/integration/select_val.rs`: the linear form costs 500.169 ns for each iteration at 16 choices and the ordered form costs 514.915 ns at the same shape, while the ordered form costs 526.238 ns at 256 choices against 984.243 ns of the linear form. The micro-measurement of `prints_the_cost_of_every_derivation` is the only point between them, at 32 choices, where the linear worst case costs 152.053 ns and the ordered search costs 34.172 ns.
- **Option:** run the end-to-end probe at 32 and 64 choices in one interleaved session, and set the bound at the highest shape where the linear walk still wins.
- **Trigger:** a session that can run end-to-end probes at 32 and 64 choices, or a measured `select_val` with 17 to 32 choices in a hot path.
- **Source:** task #545 and the section "The `select_val` dispatch decision of task [#545](https://github.com/fahchen/pbeam/issues/545)" of `docs/interpreter-dispatch-measurements.md`.

### Reuse the Y register storage of a released frame

- **Reason:** task #630 measured a free list of slot arrays for `allocate`. The release Terra key press showed no gain beyond the noise of the session, and the floor loops without `allocate` were 1.4 ns to 1.8 ns slower, so the change did not land.
- **Evidence:** a body-call probe with two `allocate` instructions in each iteration was 22 percent faster (0.76 s to 0.59 s for 3000000 iterations). The measured commit is `1c344478` of PR branch `perf/630-frame-pool`.
- **Trigger:** a measured workload that spends a large part of its time in body calls with Y registers, or a change of the run loop that removes the floor cost of the variant.
- **Source:** task #630 and the section "The Y register storage of a released frame, task [#630](https://github.com/fahchen/pbeam/issues/630)" of `docs/interpreter-dispatch-measurements.md`.

### Take the source term of a move whose source is dead

- **Reason:** task #631 measured a `move` that takes the term out of a dead source register with no reference count change. The release Terra key press showed no gain beyond the noise of the session, so the change did not land.
- **Evidence:** commit `4fc08860` of pull request [#641](https://github.com/fahchen/pbeam/pull/641) marks a `move` whose source register no later instruction reads at load time. 27,472 of the 46,835 moves of a boxed term of a Terra key press have a dead source. A probe of a boxed move gave about 1.6 ns for each take with a warm cache, or about 0.04 ms (0.3 percent) for each key press. The floor shapes run no `move` in their loop, so they do not measure the change.
- **Trigger:** a measured workload where moves of boxed terms hold a clear share of the time, or a Terra CPU harness that resolves a change below 1 ms for each key.
- **Source:** the measurement record of `docs/performance/631.md`.

### Defer the drop of a large dead value out of the instruction path

- **Reason:** task #633 split the time of the input and output path of a release Terra key press. Its largest single mechanism is the `write` call of one `get_tuple_element` of `gen_server` (position 2219 of the loaded code), about 32 µs to 34 µs two times for each key, and its probable cause is the drop of the old value of the register. A write that releases the last reference of a large value runs the drop of `Term` (`src/term.rs:517`) over each node of that value inside the instruction. ERTS pays nothing at that write: `get_tuple_element` loads one word (`erts/emulator/beam/jit/arm/instr_common.cpp:378-388`), and a minor collection copies only the live terms (`do_minor`, `erts/emulator/beam/erl_gc.c:1639`, `:1677`) and frees the young heap as one block (`erl_gc.c:1831`).
- **Option:** when a register write releases the last reference of a compound value, the process keeps the value in a list of dead values, and the scheduler drops the list when no process can run, before it waits for input. The accepted `Arc` graph of terms and `Term: Send + Sync` stay. No value or ordering that a process can see changes, because a dead value has no other reference.
- **Evidence:** the measured drop is about 63 µs to 69 µs for each key at position 2219 of `gen_server` in sessions T and U, about 0.5 percent of the run loop time of a key. The record does not measure the drops at other writes or in the modules outside the input and output group, so this figure is not an upper bound. The change moves the drop time out of the key latency, but it does not remove CPU time.
- **Trigger:** a measured workload where the drop of dead values at a register write takes a large part of its CPU time or of its latency, for example a probe that gives a drop row at or above 5 percent of a key press.
- **Source:** the measurement record of `docs/performance/633.md`.

## Runtime

### Model the process heap for GC reductions and the message copy cost

- PBeam has no process heap, and its terms keep no literal origin. A message, an ETS table, and `persistent_term` share each map with the sender, and do not copy it. Thus PBeam charges no garbage collection reductions and no message copy cost, and `maps:merge/2` of a map and its copy from a send, ETS, or `persistent_term` takes their shared nodes with no step. A change of the `message_queue_data` process flag does not move the messages and sends no adjust signal. The "Garbage collection reductions", "Scheduling", "Process flags", and "Map merge reductions" rows of `docs/erts-differences.md` record these differences, and this entry owns them.
- **Evidence:** a garbage collection costs the process at least 1 reduction, more for a large heap (`erl_gc.c:218-230`). A local send of a message of more than 64 heap words bumps one reduction for each 64 copied words (`erl_message.c:881-886`), and an alias send does the same (`erl_proc_sig_queue.c:2284-2290`). The copy counts each shared subterm once and no literal (`SHCOPY_SEND`). Measured: a send of a list of 10000 integers costs 313 reductions on pinned ERTS and 1 on PBeam. During #233, `lists:reverse/2` and `erlang:'++'/2` copy 60 elements for each reduction only while the heap has free words, and then 40 elements for each reduction to a heap fragment (`erl_bif_lists.c:111-174`, `1144-1192`). Measured: `lists:reverse/2` of 10000 elements costs 166 with a heap of 2000000 words and 2386 with the default heap. `maps:merge/2` takes a subtree that both maps share in memory, and two equal arguments, with no step (`erl_map.c:1258-1263`, `1608-1612`). Measured: `maps:merge/2` of a hashmap of 1000 keys and its copy from a send to another process and back costs 213 reductions on pinned ERTS and 0 on PBeam (#453). A change of `message_queue_data` to `off_heap` sends an adjust signal to the process after thread progress (`erl_message.c:1008-1019`, `1022-1135`; `erl_proc_sig_queue.c:3487-3518`).
- The work: a process heap with its size, garbage collection with the ERTS reductions, the heap-fragment fallback of the list BIFs, the shared and literal parts of a term for the send copy, the copy of a term in a send, ETS, and `persistent_term` for `maps:merge/2`, and the message place and the adjust signal of `message_queue_data`.
- **Trigger:** a consumer that needs exact interleaving with large messages or with processes that allocate much, or a task that adds a heap-size limit (row "Heap limit").
- **Source:** owner decision on PR #249 (#245).

### Keep the trap state of `maps:from_list/1`, `maps:from_keys/2`, and `unicode:characters_to_list/2`

- At each trap, `maps:from_list/1` and `maps:from_keys/2` hash, sort, and walk the list again to get their YCF units, and then skip the units that they already used (`from_list` and `from_keys` in `src/runtime/interpreter/bif/maps.rs`). `unicode:characters_to_list/2` builds the full list in its first call, and the traps only charge the build (`finish_list` in `src/runtime/interpreter/bif/unicode.rs`). ERTS keeps the YCF state in a magic reference (`utils.c:3733-3819`) and builds the list a part in each call (`erl_unicode.c:1342-1361`). The reductions, the trap points, and the results are the same, so only the host time changes.
- **Evidence:** on a release build, `maps:from_list/1` of 5000, 10000, and 20000 pairs takes 3, 9, and 28 ms, and the tail-recursive build of the same lists takes 1058, 4125, and 16799 ms (measured on the head of PR #249). A list of n pairs traps about n / 16000 times. `maps:from_keys/2` walks its key list in the same way, with one unit for each key (`erl_map.c:287-291`, `629-642`).
- **Trigger:** a measured path converts inputs large enough that the host time of these BIFs matters, for example after a list term change makes the list build fast.
- **Source:** Codex review of PR #249 (#245). The `maps:from_keys/2` walk comes from #445 (PR #486).

### Use a balanced multiset in `erlang:'--'/2`

- The set stages of `erlang:'--'/2` keep the multiset of the right elements in a sorted `Vec`, and `Vec::insert` and `Vec::remove` move each later element (`set_build` and `set_finish` in `src/runtime/interpreter/bif/subtract.rs`). ERTS inserts each right element into a red-black tree with one node for each element, and it removes a node when its count reaches 0 (`subtract_rbt_lookup_insert`, `erl_bif_lists.c:813-856`). Both count one insertion for each element, so the reductions, the trap points, and the results are the same. A right list of n distinct elements in descending order moves about n²/2 elements in PBeam, and these moves take no reduction. Each trap also converts the multiset and the result so far to lists for the trap state, and the next turn converts them back (`into_terms` and `from_terms`), which repeats this work for each trap.
- **Evidence:** measured on pinned ERTS and on the debug build of PBeam, with the descending right list and the ascending left list, `lists:seq(1, N) -- lists:seq(N, 1, -1)`, where the wall time includes the start of each runtime: for N = 20000, ERTS 0.22 s and PBeam 1.0 s, and for N = 100000, ERTS 0.22 s and PBeam 23.4 s. The same call with the ascending right list and the descending left list, `lists:seq(N, 1, -1) -- lists:seq(1, N)`, takes 0.34 s on PBeam for N = 100000, because the `Vec::insert` and `Vec::remove` then work at its end. The fixture case `set_build_trap` of `subtract_probe` builds an ascending list, so it does not show the cost.
- **Trigger:** a consumer subtracts two lists of more than about 10000 elements, or a measured run starts such a subtraction.
- **Source:** Codex review of PR #486 (#445).

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

- When the digits of a bignum divided by 16 are more than the reductions left, ERTS runs `integer_to_list/1` or `integer_to_binary/1` again on a dirty CPU scheduler (`bif.c:3283-3299`; `binary.c:92-108`). PBeam has no dirty scheduler. For an integer from 2^59 to 2^63 - 1 with no reductions left, it returns the runtime error `unsupported erlang:integer_to_list/1 of N with 0 reductions left: ERTS runs it on a dirty CPU scheduler`, or the same error for `erlang:integer_to_binary/1`. `erlang:integer_to_binary/2` gives the same error for a bignum whose cost in its base is more than the reductions left, for example 4 in base 2. The "Scheduling" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Trigger:** a measured path converts such an integer at the end of a budget, or a task adds dirty schedulers.
- **Source:** #234 (PR #243), #268, #399.

### Read the clock after `ERTS_CHECK_TIME_REDS` reductions

- PBeam reads the run clock and expires each due timer before it takes the next process. An ERTS scheduler reads the clock only when its processes used `ERTS_CHECK_TIME_REDS` (`CONTEXT_REDS`, 4000) reductions after the last read, and before it sleeps. Between two reads, it expires the timers on the last time that it read (`erl_process.c:59`, `9797-9810`; `erl_process.h:528-535`; `erl_time_sup.c:280`). Thus on ERTS a due timer can expire some turns later than on PBeam. The "Timers" row of `docs/erts-differences.md` records this, and this entry owns it.
- **Trigger:** a measured path depends on the turn in which a timer expires while other processes run.
- **Source:** #229 (PR #236).

### Run the other binary-match instructions

- PBeam runs `bs_start_match3`, `bs_start_match4`, `bs_get_position`, `bs_set_position`, `bs_get_utf8`, `bs_get_tail`, `bs_get_binary2` with a size in a register or a literal integer, and `bs_match` with each command that the ERTS loader accepts. `move`, `bs_start_match3`, and `bs_start_match4` share a match context that they copy to another register, and `byte_size/1` reads a match context. `swap` and `bit_size/1` of a match context stop the run with `invariant error: the match context in <Operand> reached an instruction that is not a binary match`. After a failed `=:=` command of `bs_match`, PBeam does not know the position, so a read of the match context before `bs_set_position` stops the run with `unsupported read of the match context in <Operand> after a failed bs_match =:= command`. Each other binary-match instruction, such as `bs_skip_utf8` and `bs_get_integer2`, and `bs_get_binary2` with the size `all`, stops the run with `unsupported BEAM opcode N`. The "Binary matching" row of `docs/erts-differences.md` records this, and this entry owns it. The entry "Add bitstring terms" owns the `binary` and `get_tail` commands, and the `bs_get_binary2` and `bs_get_tail` instructions, of a size that is not a multiple of 8 bits.
- **Evidence:** `bs_match_probe:bits(<<165>>)` stops with `unsupported bs_match binary of 3 bits: needs a bitstring term`. The test `gives_a_runtime_error_for_a_read_after_a_failed_equal_command` of `tests/runtime/interpreter.rs` stops at a `skip` command after a failed `=:=`.
- **Trigger:** a measured path matches a binary with one of these instructions, gives a match context to `swap` or `bit_size/1`, or reads a match context after a failed `=:=` command without `bs_set_position`. Add only the instructions that the path needs.
- **Source:** #219 (PR #227), #395, #396, #421.

### Run the unsupported BIFs that OTP code calls

- The "Unimplemented BIFs" row lists the `unsupported` BIF table entries that OTP code often calls. Each returns `unresolved external MFA M:F/A`. The groups:
  - Timers: `erlang:start_timer/4`, `erlang:send_after/3,4`, `erlang:cancel_timer/1`, and `erlang:read_timer/1,2`. `erlang:start_timer/3` and `erlang:cancel_timer/2` already use the timer list of #150 (PR #172).
  - Aliases: `erlang:alias/1`, which `erlang:alias/0` calls, and `erlang:unalias/1`.
  - Equality: `erlang:'/='/2`. It gives other results than `=/=` only for floats (see "Add float terms").
  - Persistent terms: `persistent_term:erase/1`, `persistent_term:put_new/2`, and `persistent_term:info/0`.
  - Signals: `erlang:exit/3`.
  - Maps: `maps:get/2` and `maps:is_key/2` through a fun call. The compiler changes a direct call to the `map_get` and `is_map_key` BIFs, which PBeam runs.
- **Trigger:** a measured path calls one of them. Add each group in its own task.
- **Source:** audit of the BIF table at the M2 close.

### Run the remaining test instructions for `io:format/2`

- #662 added `is_any_native_record`, so `io:format/2` with `~p` of an atom, an integer, a float, a list, a tuple, or a map writes as ERTS does under the PBeam boot. `~p` of a binary that is not empty stops with `unresolved external MFA erlang:bit_size/1` in `io_lib_pretty:print_length_binary/7` (io_lib_pretty.erl:1116) (measured). #359 added `is_bitstr`, #399 added `is_ne`, and #302 added `is_port`, which stopped `~w` of a list, a tuple, or a map before (io_lib.erl:820, 823). The "Unimplemented instructions" row records `get_record_elements` and `put_record`. A format without a term argument, `~w` of an integer, a list, a tuple, a map, or a binary, and `~s` of a string already work. PR #188 also named `net_kernel:dflag_unicode_io/1` and `erl_scan:reserved_word/1`. The first is a BIF that PBeam runs (PR #88), and the second is a function of `erl_scan.beam`, which a run gives with the other OTP modules.
- **Trigger:** a consumer or a probe prints a compound term with `io:format/2`, for example to compare its output byte for byte through the `pbeam` binary.
- **Source:** PR #188 (#179) and measured probes at the M2 close.

### Turn on native bindings only after `erlang:load_nif/2`

- The native bindings of `prim_tty` are active from module load, also before `erlang:load_nif/2` and after a failed load. ERTS replaces a stub body only after a successful load (`erl_nif.c:4990-5007`). The "NIF stubs" row records this. PR #132 proposed the task "[m2-e1] Turn on native bindings only after erlang:load_nif/2", but it was not filed.
- **Trigger:** a path calls a `prim_tty` NIF before `erlang:load_nif/2` or after a failed load, or a second module gets native bindings.
- **Source:** PR #132 (#127).

### Poll fd 0 while processes run

- PBeam waits for the read select of fd 0 and for the pipes of the ports only when no process can run (#299, #302). ERTS also waits for it in a poll thread while processes run (erl_process.c:3339-3372), so input arrives while other processes run. Thus in PBeam, a run in which a process can always run never gets the `{select, TTY, Ref, ready_input}` message. The "Input select" row of `docs/erts-differences.md` records this.
- The work: a poll of fd 0 with no timeout at a scheduler switch point, for example after a number of reductions, as an ERTS scheduler checks its own poll set (erl_process.c:9946-9960).
- **Trigger:** a program reads standard input or the output of a port program while one of its processes can always run, for example a busy worker.
- **Source:** PR for #299.

### Run the other port options and port kinds

- `open_port/2` runs only `{spawn_executable, Name}` with `binary`, `exit_status`, `use_stdio`, `nouse_stdio`, `stderr_to_stdout`, and `{args, Args}` (#302). Each other option, the list data mode, the port kinds `spawn`, `spawn_driver`, and `fd`, the port BIFs other than `port_command/2`, `port_close/1`, and `port_info/1`, and a message, link, monitor, exit signal, or name of a port give a runtime error (row "Ports" of `docs/erts-differences.md`).
- `port_command/2` writes with a blocking write; ERTS queues the data that does not fit in the pipe and marks the port busy (sys_drivers.c:1140-1203). A port program keeps each fd of PBeam that is not close-on-exec. After `os:set_signal(sigchld, ignore)`, the host reaps each program, and a port with `exit_status` returns a runtime error; ERTS reaps its programs in `erl_child_setup` (erl_child_setup.c:396-420). For the same reason, the `pbeam` binary keeps `SIG_DFL` for `SIGCHLD` at the start, where ERTS sets `SIG_IGN` (row "OS signals"). A reaper process as `erl_child_setup` would remove both differences.
- **Trigger:** a consumer uses another option, port kind, or port BIF, for example `{line, L}` or `Port ! {self(), {command, Data}}`, or writes more data than the pipe holds to a program that writes back, or reads the exit status of a port after `os:set_signal(sigchld, ignore)`.
- **Source:** PR for #302.

### Run the break menu and the crash dump of `SIGINT` and `SIGUSR1`

- After #301, `SIGINT` stops a `pbeam` run with `pbeam: unsupported break of SIGINT: PBeam has no break handler` and status 1, and `SIGUSR1` stops it with `pbeam: unsupported erlang:halt/2 with a crash dump slogan: PBeam writes no crash dump` and status 1. ERTS writes the break menu and reads a key from fd 0 (break.c:571-687), and for `SIGUSR1` writes `Received SIGUSR1` and `erl_crash.dump` and exits with status 1 (erl_signal_handler.erl:39-41, bif.c:4370-4387). The "OS signals" row records this.
- **Evidence:** `tests/feature/host_signal.rs` (`gives_a_runtime_error_for_a_crash_dump_and_a_break`): with stdin `/dev/null`, ERTS writes the break menu and exits with status 0 at the end of file of fd 0.
- The work: the break menu with its keys, which needs the process, port, and table information that ERTS prints, and a crash dump writer.
- **Trigger:** a consumer needs the break menu at `^C` or a crash dump.
- **Source:** PR for #301.

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
- On Linux, the `inet_db` start calls `inet:gethostbyname/1` for the host name when the domain is empty (inet_config.erl:112-134). Before `kernel_safe_sup` starts, `inet_gethost_native` spawns `run_once/0` (inet_gethost_native.erl:575-577). In PBeam, `init:get_argument(bindir)` gives `error`, so that process exits with `{badmatch,error}` (inet_gethost_native.erl:416). `inet_config` ignores the error result, so the domain stays empty. In ERTS, the `inet_gethost` port program resolves the name. When the result is a long name, `set_hostname/1` sets the domain (inet_config.erl:122-127). The port also needs the `{packet,4}` and `eof` options of `open_port/2`. Observed in PR #717 (#449).
- **Trigger:** a consumer reads one of these variables or flags, for example `init:get_argument(root)`.
- **Source:** PR #191 (#185).

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

### Compare a shared map root before walking a map

- **Reason:** the map arm of `Term::eq` (`src/term.rs:586-593`) zips the pairs of two maps onto a work stack and compares the lengths, with no check of a shared root, and no measurement harness covers a comparison of two maps with the same root. The unit test `tests/term.rs:395` compares a map with its own clone and does share the root, but it is a correctness test and not a measured consumer.
- **Evidence:** the map comparisons that run in a loop are the per-cell loop of Terra `Renderer.diff/2` over 4-key cell maps, which each frame builds afresh, and the microbenchmark `term_eq_map_1000_equal` (`tests/term/data_structure_costs.rs:461`), which builds two maps of 1000 keys independently. Every other comparison site is a one-shot path or a correctness test, and no fixture and no measurement harness compares a map against a clone, a received copy, or a value read back from a table. The pinned ERTS skips an identical root (`erts/emulator/beam/utils.c:1133`) and identical HAMT children (`:1495`), so the check follows the reference, but on these consumers the roots differ and the check saves nothing.
- **Trigger:** a measured consumer that compares two references to one map value: a map read back from ETS or a persistent term, a map sent and compared against a retained copy, or a `lists:member/2` over a list that holds the same map twice.
- **Source:** the consumer audit of the term proposal on #552, and issue #555.

### Share map nodes in the results of the map BIFs

- `maps:remove/2`, `maps:take/2`, and `maps:merge/2` build their result map again with `Map::from_pairs`, so the result holds no node of an argument. An update that puts the value that the key already has makes a new map. ERTS shares the nodes in each of these cases, so `maps:merge/2` of such a result and its argument uses more reductions on PBeam. The row "Map merge reductions" of `docs/erts-differences.md` records this difference, and this entry owns it.
- **Evidence:** `hashmap_delete` copies only the path of the key (`erl_map.c:2881-3184`). `hashmap_merge` gives a node with keys from only one argument as the result (`erl_map.c:1768-1778`). An update returns the same map for a key that already has the same value word (`erl_map.c:2083`, `2219-2222`, `2523-2526`, `2608-2611`). Measured on the pinned ERTS for a hashmap `M` of the keys 1..1000, in reductions of `maps:merge/2`, ERTS then PBeam: `M` and `maps:remove(500, M)`: 5 and 213; `M` and `maps:merge(M, #{2000 => x})`: 5 and 213; `M` and `M#{500 => 500}`: 0 and 5 (#453).
- **Trigger:** a consumer that needs the exact reductions of `maps:merge/2` on such maps, or that removes keys of a large map in a loop.
- **Source:** PR for #453.

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

### Read code files from the bundle as a read-only code path root

- In interactive mode, a bundle is a read-only code path root. `prim_file` reads a path under this root from the bundle, not from the host file system. Thus `erl_prim_loader` and `code_server` run the pinned code path search on a bundle as on a directory (design section 13 of `docs/code-loading.md`). The bundle format does not exist yet, so no task can define the root or its paths now.
- **Trigger:** the bundle format of [#15](https://github.com/fahchen/pbeam/issues/15) lands.
- **Source:** design `code-loading.md` of Suikou review 01a1282e, accepted by the owner on 2026-10-11 (#441).

### Hold the part of the result of a YCF map BIF across a yield

- `maps:keys/1` and `maps:values/1` (`src/runtime/interpreter/bif/maps.rs`) take the whole unit vector before the walk, and the last call of the trap chain builds the whole result list with no yield inside it. ERTS keeps the iterator and the part of the result in the trap state and checks the budget inside the loop (`erl_map.c:2848-2878`, `utils.c:3733-3819`).
- **Evidence:** the Codex review of PR #753. A map of N keys allocates a unit vector of N entries at each call, also at each resume, and the build of the result of a large map is one O(N) step that no reduction covers.
- **Risk:** the one scheduler thread runs the build of a large map without a switch.
- **Trigger:** a measured path with a map of about 100000 keys or more, or a task that changes the YCF trap state of these BIFs.
- **Source:** Codex review of PR #753 (#750).

## Exceptions

### Give the `error_info` cause of BIF errors

- PBeam gives the `error_info` map `#{module => Format}` with no `cause` for each BIF error except `erlang:spawn_opt/4`. ERTS adds `cause => Cause` when the BIF raises with `EXF_HAS_EXT_INFO` (`beam_common.c:993-1002`), for example `none` or `notalive` for `register/2` (`bif.c:2310-2316`), `badopt` for `monitor/3` (`bif.c:988-995`), and the causes of the `ets` BIFs (`erl_db.c:794-931`). `erl_erts_errors:format_error/2` and `erl_stdlib_errors:format_error/2` read the cause to write the error text.
- **Evidence:** `catch register(1, self())` gives `[{error_info, #{cause => none, module => erl_erts_errors}}]` on the pinned ERTS and `[{error_info, #{module => erl_erts_errors}}]` on PBeam (`registers_a_name_or_raises_badarg_as_pinned_erts` in `tests/runtime/interpreter.rs`).
- **Trigger:** a consumer reads the cause of a BIF error, for example the error text of a shell or a logger report, or OTP code that matches the cause.
- **Source:** [#715](https://github.com/fahchen/pbeam/issues/715).

### Give the stack trace of a BIF error after a trap

- A BIF that traps to a BIF export (`BifResult::TrapBif`) continues through `PendingBif`, and an error after the trap gets the stack trace `[]`. ERTS gives the BIF frame with the arguments in the registers, which are BIF-specific: `lists:member/2` gives the rest of the list (`[x, Rest]`), and `length/1` puts the first list back (`erl_bif_guard.c:243-247`). The PBeam trap arguments are internal continuation state (for example `[Rest, Count]` of `length/1`, three arguments of `erlang:'++'/2`), and a copy of them breaks the owned output of `list_to_bitstring_continue`.
- **Evidence:** on the pinned ERTS, `lists:member(x, lists:seq(1, 100000) ++ a)` raises `badarg` with the first frame `{lists, member, [x, Rest], [{error_info, #{module => erl_stdlib_errors}}]}`; PBeam gives `[]`. Codex review of PR #736.
- **Trigger:** a consumer reads the stack trace of a BIF error after a trap.
- **Source:** [#715](https://github.com/fahchen/pbeam/issues/715).

### Give caller frames and line locations in stack traces

- PBeam records no line table, so `erlang:error/1,2` gives the frame of its caller with the location `[]` and the frames of the process after it. ERTS records the location of each frame. PBeam matches ERTS only at `backtrace_depth` 0 or 1, and only for code with no line table.
- The error handler path of [#447](https://github.com/fahchen/pbeam/issues/447) adds the frame of the caller of `error_handler:crash/1` to the frames of `erlang:error/1` in about one of four calls. The reductions of the error handler code match ERTS in a run with a live code server; a run of the library API has no code server and charges the extra reductions of the `init:ensure_loaded/1` round trip.
- **Trigger:** a consumer needs the location of a frame, or the exact frames of an `undef` exception.
- **Source:** owner decision of 2026-10-11 (Suikou review 01a1282e, AFK log second batch); [#446](https://github.com/fahchen/pbeam/issues/446), [#447](https://github.com/fahchen/pbeam/issues/447), PR [#473](https://github.com/fahchen/pbeam/pull/473).

### Raise `undef` for an entry module that the run does not load

- After #42 (PR #194), the `pbeam` binary accepts only an argument atom that a loaded module holds and stops with `argument <M> is not an atom of the loaded modules`, and it stops with `missing BEAM module M` for an entry module that the start BEAM files do not hold. #447 runs the pinned `error_handler` for a call of a module that the run does not load, so the undefined-module path of `error_handler` exists now. The remaining work is a `BeamProgram` API that adds the atom of the entry module and of each argument, and an entry that loads its module through the error handler. The "Run arguments" and "Undefined functions" rows record the difference.
- **Trigger:** a consumer runs an entry module, or gives an argument atom, that the start BEAM files do not hold.
- **Source:** review of #42; the remaining work of [#447](https://github.com/fahchen/pbeam/issues/447).

### Halt the PBeam boot with the `init` crash text

- After #6 (PR #195), the `pbeam` binary writes `pbeam: root process exited with <Reason>` for a root exit. Under `erl -noshell -eval`, ERTS writes `Error! Failed to eval: ...` and then `Runtime terminating during boot (<Reason>)` through `init:crash/2` and `erlang:halt/1` (init.erl:665-679, 1552-1567). Doing the same needs `erlang:halt/1` with a string, which writes the slogan and a crash dump (`erlang:halt/2` with an integer status runs since #301), `erts_internal:term_to_string/2` (it can use the `print_term` of #6), and an `init` process that watches the boot process.
- **Trigger:** a consumer needs the ERTS crash text byte for byte.
- **Source:** design of #6.

### Send the uncaught-exception report to `logger`

- PBeam writes no report for an uncaught exception. For an uncaught `error` or `throw`, ERTS sends `{log, error, "Error in process ~p with exit value:~n~p~n", Args, Metadata}` to the registered `logger` process (beam_common.c:697-718, erl_trace.c:2812-2820). The "Error report" row records this. `pbeam_boot` starts `logger` (PR #188), so the receiver exists. PR #188 said that #73 tracks this work, but #73 is closed.
- **Trigger:** a consumer needs the report of a process that crashes.
- **Source:** PR #188 (#179).

### Value checks of `process_flag/2` and `system_flag/2`

- After #155 (PR #177), `max_heap_size` (`erl_gc.c:3824-3872`, needs the process minimum heap size), `{monitor_nodes, Options}` (`dist.c:6732-6799`), a `scheduler` value above the scheduler count, and the `system_flag/2` sub-checks of `internal_cpu_topology`, `erts_alloc`, `sequential_tracer`, `schedulers_online`, and `dirty_cpu_schedulers_online` give the runtime error where ERTS raises `badarg`. The "System flags" row of PR #177 records them.
- **Trigger:** a consumer sets one of these flags.
- **Source:** review of #155.

### Check the ordered map iterator crash of ERTS at each OTP pin change

- Pinned ERTS 29.1.1 crashes with SIGSEGV in `erts_internal:map_next/3` when an ordered iterator path holds a key that the map does not hold, or ends with a tail that is not `[]`. It crashes in about one run in four, and in every run with `MallocScribble=1`. The BIF raises `badarg` with a half-built tuple left in its heap fragment, and the GC after the BIF reads the unset element (`erl_map.c:3670-3707`). OTP master has the same code. The "Map iteration" row of `docs/erts-differences.md` records it (PR #177). `tests/integration/logger_start.rs` checks this input on PBeam only (`returns_badarg_for_an_ordered_path_that_crashes_pinned_erts`), and the live `erl` reference run leaves it out. The owner decided not to report it to OTP.
- **Trigger:** a change of the pinned OTP version. Run the reproduction again. If ERTS raises `badarg`, put the input back into the live reference run of `logger_start_probe:ordered/0`, remove the PBeam-only test, and change the row.
- **Source:** investigation of a flaky reference test in the review of #155.

## Capability security

### Add a lease to a capability

- After the design of #310, a capability stays valid until a revocation or the exit of its holder. A lease ends a capability at a set time.
- **Reason:** the owner put the lease in the backlog in the design review of #310 (question 7). The revocation signal of #336 covers the current needs.
- **Option:** an end time on the run clock `Timers::now` (`src/runtime/timer.rs`), which is the host monotonic `Instant` from the run start. The host system clock can go back, so it is not a good lease clock.
- **Trigger:** a consumer must take back authority at a set time without a revoker.
- **Source:** design review of #310 (PR #314).

### Limit resources with budgets and quotas

- Transition 2 has no budget gate. The "First budget gate" choices of `docs/capability-security-migration.md` are open: the quota unit, the reservation ownership, the excess behavior, and the accounting of shared allocations (questions 9 to 12 of the design review of #310).
- **Evidence:** at its process limit, ERTS raises `error:system_limit` from the spawn BIF and sends "Too many processes" to the logger (`erts/emulator/beam/erl_process.c:12537-12540` at `OTP-29.1.1`). A probe with `erl +S 1 +P 1024` gives `system_limit` from `spawn` and `spawn_link`. This is the preferred excess behavior of a process quota. The migration contract requires that a child refers to a shared parent account and never copies a full budget.
- **Trigger:** a consumer must limit the resources that a guest uses.
- **Source:** design review of #310 (PR #314).

### Delegate a capability from a process that is not the parent

- After the design of #310, a process gets a capability only from its parent at spawn (#334), from the launcher, or from the policy process at run time (#337).
- **Option:** a broker process gives one of its capabilities to a new child of another process, after the broker agrees explicitly.
- **Reason:** no consumer needs it. A run-time grant of the policy process covers most requests for a capability from another source.
- **Trigger:** a real consumer that a run-time grant of the policy process cannot serve.
- **Source:** design review of #310 (question 17, PR #314).

### Add realm and incarnation identity to the subject

- After the design of #310, the subject is the process, and a run has one realm. The security context lives as long as the process, and no check finds a capability by the pid number. The delegation records hold pids, and PBeam never gives a pid number again (`Scheduler::next_pid`), so a record cannot name a later process.
- **Reason:** no consumer reads a realm or an incarnation.
- **Trigger:** a host that runs plugins that do not trust each other, each with its own policy; a lookup of authority by pid; PBeam uses process table slots again as ERTS does; or completion routing of transition 6 that needs an incarnation.
- **Source:** design review of #310 (question 2, PR #314).

### Add a text syntax for capability scopes

- After the design of #310, the policy code examines the operation name and the arguments, and each capability kind has data constraint fields. The runtime needs only the operation and a summary of the arguments, so no scope syntax exists.
- **Reason:** no consumer reads a scope from text.
- **Trigger:** the launcher and manifest work (M8) must read capability constraints from a manifest or a configuration file, or a policy that is not BEAM code.
- **Source:** design review of #310 (question 4, PR #314).

### Wake the scheduler for a launcher revocation

- After #336, the run calls `Policy::revocations` before each scheduling step (`BeamProgram::run_root` in `src/runtime/interpreter.rs`). While each process waits, `Scheduler::wait_events` blocks until a timer, a port, a terminal, a dirty IO call, or a host signal wakes it. With none of them, the run ends. Thus a revocation that a host thread gives while each process waits takes effect only at the next of these events, or never.
- **Option:** a wake handle of the launcher, for example a pipe or a socket that `Scheduler::wait_events` also polls, as the notify pipe of the host signals (`signal::notify_fd`). A host thread writes to it after it puts a kind in its queue.
- **Reason:** no consumer revokes from another thread. The only launcher that revokes is the test policy of `tests/integration/revoke.rs`, which revokes while the root process runs.
- **Trigger:** a host that revokes from another thread or at a set time, for example the launcher and manifest work (M8).
- **Source:** Codex review of PR #357 (#336).

### Bound the work of one revocation

- After #336, one revocation signal, and one `pbeam_cap:revoke/2` call, go through all the released records below the copy and send a signal to each live downstream copy in one step (`Delegations::revoke` in `src/runtime/capability.rs`). The signal counts 1 and the BIF counts 1 reduction, and neither yields. The cost row of `docs/effect-inventory.md` is `O(r)`, as the design of #310 accepts ("The cost of a revocation increases with the number of holders").
- **Evidence:** ERTS counts more than 1 for a signal that does more work (for example `cnt += 4` for a NIF select, `erl_proc_sig_queue.c:6955` at `OTP-29.1.1`), and an exiting process sends its link signals in parts with a reduction budget (`ERTS_CONTINUE_EXIT_LINKS`, `erl_process.c:14545-14554`). ERTS has no revocation, so no ERTS count applies.
- **Option:** a count for each record that the revocation reads, or a walk that yields and continues in the next turn.
- **Reason:** a delegation tree with many released records or many live copies below one copy has no consumer yet. With one scheduler, the other processes wait only for the time of the walk.
- **Trigger:** a consumer with large delegation trees, or the multiple schedulers of M10.
- **Source:** Codex review of PR #357 (#336).

### Publish a shared policy table that checks read

- After #337, the policy process runs `initial/1` and `grant/3`, and a check reads only the copies of the caller (`SecurityContext::check` in `src/runtime/capability.rs`). The design of #310 also has a shared policy table: the policy process publishes data, like `persistent_term`, and a check reads it with no lock and no message (section "Policy module, policy process, and policy table" of `docs/capability-security-migration.md`).
- **Evidence:** ERTS replaces the `persistent_term` table pointer on a write and frees the old table after thread progress (`erts/emulator/beam/erl_bif_persistent.c:187`, `773-779` at `OTP-29.1.1`).
- **Reason:** no check reads policy data yet. A table without a reader would be a foundation with no consumer.
- **Trigger:** the first check that reads policy data, for example an audit rule of #338 or the opt-in check-time extension.
- **Source:** implementation of #337.

### Revoke a grant that the holder did not handle yet

- After #378, `pbeam_cap:policy_revoke/2` revokes the copies of a kind that have a delegation record when it runs (`Scheduler::policy_revoke` in `src/runtime/scheduler/policy.rs`). A copy of a grant gets its record when the requester handles the grant signal (`Signal::Grant` in `src/runtime/scheduler/signals.rs`). Thus a revocation in a later call of the policy code does not revoke a grant that the requester did not handle yet, and the requester then holds the copy.
- **Evidence:** with one scheduler, the grant signal puts the requester in the run queue before the next call of the policy process (`Scheduler::queue_signal` and `Scheduler::begin_policy_call`). The requester handles the signal in its turn when its signal queue does not hold more signals than the turn can handle.
- **Option:** give the copy its identifier and its record when the policy process sends the grant signal, and release the record when the requester exits before it handles the signal. Then the revocation signal comes after the grant signal in the queue of the requester.
- **Reason:** no policy revokes a grant in the call after the grant, and no consumer fills the signal queue of a requester that waits for its grant.
- **Trigger:** the multiple schedulers of M10, or a policy that revokes a grant in the call after the grant, for example an audit rule of #338.
- **Source:** implementation of #378.

### Limit the targets of `process` `send`

- After #370, the `process` capability with `send` permits a send to each process: the send path checks the operation only (`deliver_to` in `src/runtime/interpreter/bif/erlang.rs`). It has no constraint field.
- **Option:** a constraint of the targets, for example the pids of the own tree or the registered names of a list.
- **Reason:** the owner decided an all-or-nothing `send` in Suikou review 01a12608 and put a target constraint in the backlog. No consumer limits the targets yet.
- **Trigger:** a policy that must keep a guest from a send to a process outside its group, for example an audit rule of #338.
- **Source:** Suikou review 01a12608 (#368).

### Add a `name` capability kind

- After #368, `erlang:register/2`, `erlang:unregister/1`, and `erlang:whereis/1` are `ambient` in `docs/effect-inventory.md`. A send or a monitor by name checks only the `process` capability.
- **Option:** a `name` kind with the operations `register` and `lookup` and the constraint field `names`.
- **Reason:** the owner decided not to add the kind until a consumer needs it (Suikou review 01a12608).
- **Trigger:** a policy that must limit the names that a guest registers or finds.
- **Source:** Suikou review 01a12608 (#368).

### Add an `ets` capability kind

- After #368, each `ets` BIF is `ambient` in `docs/effect-inventory.md`. The access mode of a table (`private`, `protected`, or `public`) controls the access, as on ERTS.
- **Option:** an `ets` kind for the creation of a table, with a constraint of the names of named public tables.
- **Reason:** the owner decided that ETS has no check until a consumer needs one (Suikou review 01a12608).
- **Trigger:** a policy that must limit the tables that a guest makes or the public tables that it uses.
- **Source:** Suikou review 01a12608 (#368).

### Add a `clock` capability kind for a deterministic sandbox

- After #368, `erlang:monotonic_time/1` and `os:system_time/1` are `ambient` in `docs/effect-inventory.md`. A clock is a source of nondeterminism, not an authority, and OTP code reads clocks often, also in the process of the caller.
- **Option:** a `clock` kind that a deterministic sandbox holds back, or a run clock that the launcher gives.
- **Reason:** no consumer needs a deterministic run yet.
- **Trigger:** a sandbox that must give the same result for each run of the same input.
- **Source:** Suikou review 01a12608 (#368).

### Check `erlang:list_to_pid/1` when it is implemented

- `erlang:list_to_pid/1` is not implemented. It makes a pid of any process from text, also of the policy process, so it must have a check. The decisions of #337 also say that the task that implements it must keep the policy process out or check `process` `exit_other` for trusted processes (section "Policy module, policy process, and policy table" of `docs/capability-security-migration.md`).
- **Reason:** the owner put the check in the backlog in Suikou review 01a12608, because the BIF has no implementation yet.
- **Trigger:** the task that implements `erlang:list_to_pid/1`.
- **Source:** Suikou review 01a12608 (#368).

### Permit the other IO requests at the IO service entry

- After #463, the IO service entry of `user` and `standard_error` permits only the output requests of a runtime process: `{put_chars, Encoding, Chars}`, `{put_chars, Encoding, io_lib, Function, Arguments}` for `format`, `fwrite`, and `write`, and `{requests, Requests}` of such requests (row "Boot authority" of `docs/erts-differences.md`). It denies each other request, for example `{get_line, Encoding, Prompt}` of `io:get_line/1`, `{setopts, Options}`, `getopts`, and `{get_geometry, Geometry}`.
- **Option:** permit the input requests (`get_chars`, `get_line`, `get_until`) for a sender that holds `terminal` `read`, and `getopts` and `get_geometry` for a sender that holds `terminal` `query`. A request with an MFA, such as `get_until`, needs the same protection as the `io_lib` MFA of `put_chars`, because `group` applies it in its own process.
- **Reason:** no restricted guest reads from the terminal or reads its options yet. The design denies each request that has no consumer (Suikou review 01a1282e).
- **Trigger:** a policy run whose guest reads from the terminal, or reads the terminal options or geometry.
- **Source:** design of #461, implementation of #463.

### Add a service entry for `file_server_2`

- After #462, the runtime to boot gate denies each message from a runtime process to `file_server_2`, a boot process. Thus `file:read_file_info/1` and `os:find_executable/1`, which call `file_server_2` (`lib/kernel/src/file.erl:665-666`, `3024-3029` at `OTP-29.1.1`), exit in a restricted guest: `gen_server:call/3` exits with `{{capability_denied, Stack}, {gen_server, call, Arguments}}` (`lib/stdlib/src/gen_server.erl:1296-1302`).
- **Option:** register `file_server_2` with `pbeam_cap:serve/2` as a file service, and permit each request whose file operation the `file` capability of the sender permits.
- **Reason:** the `file` capability kind does not exist yet (#390).
- **Trigger:** the `file` capability kind.
- **Source:** design of #461, implementation of #463.

### Give a trusted guest direct access to boot processes

- After #462, the runtime to boot gate denies each message from a runtime process to a boot process by its pid or its name, and each link, exit signal, group leader change, inspection, timer, and `erlang:unregister/1` that targets a boot process (row "Boot authority" of `docs/erts-differences.md`). Only a monitor and a message to an alias of a boot process pass. Thus a guest in a run with `--boot` cannot call a boot service that has no service entry, for example `init:get_argument/1`, which sends a request to `init`, or `logger`, which calls `logger_server`.
- **Option:** a capability, for example of a new kind `boot`, that permits a trusted guest the operations of the gate on boot processes.
- **Reason:** the owner decided that the gate denies by default (Suikou review 01a1282e, R2), and no policy run needs such a service yet.
- **Trigger:** a policy run needs a boot service that has no entry, such as `init:get_argument/1` or `logger`.
- **Source:** design of #461, implementation of #462.

### Spawn a runtime process from a boot process

- After #462, a boot process that spawns with the option `{capabilities, Map}` gets `error:capability_denied` (`SecurityContext::delegate` in `src/runtime/capability.rs`). A boot process has no capability copies to reduce, and only `pbeam_cap:enter_runtime/2` of the root process makes a runtime process.
- **Option:** the boot spawn makes a runtime child with the capabilities of `Map`, with the source `launcher`, so that a launcher can start more than one application root.
- **Reason:** no consumer needs it yet. The design keeps the denial fail-closed (Suikou review 01a1282e).
- **Trigger:** the launcher of [m8].
- **Source:** design of #461, implementation of #462.

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

### Fail a test that leaks in nextest

- nextest shows a test as `LEAK` when a process that the test started keeps the stdout or stderr of the test after the test ends. The default result of a leak is pass, so the merge gate does not stop a new leak. A `leak-timeout` with `result = "fail"` in `.config/nextest.toml` makes a leak a failure. After #330, a nextest run of the full suite shows no leak.
- **Risk:** the default leak period is 100 ms. A program that ends slowly on a loaded CI runner can make a test fail at random.
- **Trigger:** a new `LEAK` in a nextest run, or an owner decision to gate leaks.
- **Source:** PR for #330.

## Future features

### Suspend, save, and resume any process

- **Idea:** stop any process at a safe point and write its whole state to a portable form: the registers, the stack frames with their code positions, the message queue, the process dictionary, the links and monitors, and the module versions that the process runs. Include a dump of the instruction code of those module versions, so that the saved state does not depend on the code path of the run that resumes it. Later, resume the process in the same run or in another `pbeam` run.
- **Why PBeam can do this:** the interpreter keeps the state of a process in safe Rust values (the `Arc` graph of terms, and frames with a module id and an instruction index), not in native stack frames or JIT code. The one scheduler gives a safe point at each switch-out.
- **ERTS:** ERTS has no such feature. `erlang:suspend_process/1,2` and `sys:suspend/1` stop a process only in the same node, and `erlang:hibernate/3` drops the stack. A saved process is a PBeam extension, so it must stay out of the ERTS-compatible path: give it its own API with a capability check.
- **Open questions:** the identity of pids, references, ports, and timers after a resume; the resources that cannot move (ports, sockets, NIF resources, and ETS tables that the process does not own); a check that the saved code matches; and the capabilities that a saved state carries.
- **Trigger:** an owner decision after M10, or a consumer such as live migration of a process, debugging, or a test that replays a run.
- **Source:** owner request in chat, 2026-10-16.
