# Terra reference run and compatibility inventory

This document records the Terra Counter reference run on the pinned ERTS and the operations that the run needs. [Task #3](https://github.com/fahchen/pbeam/issues/3) owns it. The [Terra target](terra-target.md) gives the scenario contract. [Task #21](https://github.com/fahchen/pbeam/issues/21) compares a PBeam run with this inventory.

## Reference command

Run this command from the repository root on macOS:

```sh
LANG=en_US.UTF-8 mise run terra-reference
```

The task runs `scripts/terra_reference.sh`. The script does these steps:

1. It runs the command of the [Terra Counter example](../examples/terra_counter/README.md), `bash scripts/example.sh terra_counter --erts`, in a pseudo-terminal with `scripts/terra_reference.escript`. This command builds the example and runs `TerraCounter.main/0` with `erl +S 1 -noshell`. The build downloads the pinned Terra commit into `target/terra/<commit>/`, or uses the local repository in `PBEAM_TERRA_SRC` (`scripts/terra.sh`). The Elixir compiler runs the macros of the Terra sources, so the build runs third-party code. The harness writes `target/reference/run/report.txt`.
2. It writes the Terra commit, the `mix.lock` blob, the `erl` path, the OTP and ERTS versions, the Elixir and Rust versions, and the macOS version to `target/reference/versions.txt`.
3. It runs the same scenario again in the pseudo-terminal through `terra_calls:run/2` (`scripts/terra_calls.erl`). This run uses the code path of the build (`target/examples/terra_counter/erts.paths`) and turns on the ERTS call counters. The harness writes `target/reference/inventory/report.txt`, and `terra_calls` writes `target/reference/inventory/calls.eterm`.
4. `scripts/terra_inventory.escript` writes the static candidates and the exercised operations to `target/reference/inventory.txt`.

The exit status is 0 only when both runs pass each check. It is 1 when a check fails, and 2 when the platform is not macOS or the script does not start in the repository root.

The task compiles the upstream `examples/counter_app.exs` with the Terra sources and runs `Terra.run(Counter)` from the compiled entry `examples/terra_counter/terra_counter.ex`. It does not run the upstream command `mix run examples/counter.exs`. Mix needs the Hex package manager to read the development dependency `ex_doc` of the Terra `mix.exs`, and `mise.toml` pins neither. The entry sets `standard_io` to binary mode, as the start of the `elixir` application does in `mix run` (see the entry module).

### Pseudo-terminal harness

`scripts/terra_reference.escript` opens the macOS `script(1)` utility through an Erlang port: `script -q /dev/null /bin/sh -c ...`. The port pipes are the stdin and stdout of `script(1)`, so the harness sends the keys and reads the bytes that the program writes to the terminal. The environment has `TERM=xterm-256color`, `LANG=en_US.UTF-8`, and `LC_ALL=en_US.UTF-8`.

The shell in the pseudo-terminal sets 80 columns and 24 rows with `stty`. It writes `stty -g` and `stty -a` before and after the command, and it writes the exit status of the command.

A screen model applies the control sequences of the output to a grid of 80 by 24 cells, and of 30 by 100 cells after the resize: cursor movement, erase, SGR attributes, the alternate screen of mode 1049, and the cursor visibility of mode 25. The report lists each control sequence that the model does not know.

The harness sends one key only after the visible screen shows the expected text of the step before it:

| Step | Key | Expected visible text | Bound |
| --- | --- | --- | --- |
| Start | None | `Count: 0` and `j/k to change, q to quit` | 600 seconds, because the command can build the example first |
| Resize | None; sets the pseudo-terminal to 30 rows and 100 columns | `Count: 0` | 10 seconds |
| Flush | `\e[`, then 500 milliseconds | `Count: 0` | 10 seconds |
| Increment | `j` | `Count: 1` | 10 seconds |
| Increment | `j` | `Count: 2` | 10 seconds |
| Decrement | `k` | `Count: 1` | 10 seconds |
| Quit | `q` | The command exits. | 10 seconds |

The resize step writes the `resize` file, and a background loop of the shell of the pseudo-terminal sets 30 rows and 100 columns with `stty`. The harness waits until the loop writes the size and a marker, and accepts only `30 100`; a resize that fails fails the step. Terra caches the terminal size when it enters the terminal (`lib/terra/terminal/owner.ex:192`), and `Owner.info/0` gives that cache (`229-236`), so `Terminal.size/0` keeps 80 by 24 and the view does not change. The step also checks that the run continues after the signal; it does not show that the signal handler ran, because Terra changes no view. The flush step sends an incomplete escape sequence and waits 500 milliseconds, so the input flush timer of Terra (`lib/terra/input.ex:211-222`) fires; the step checks that the run continues after the timer.

A step that does not see its text in its bound fails the run. A timeout kills the process of the pseudo-terminal, which closes the terminal and sends `SIGHUP` to the program. After the run, these checks must pass: the exit status of `script(1)` and of the command is 0, the output has `{:ok, 1}`, the main screen is active, the cursor is visible, the SGR attributes are reset, and `stty -g` after the run is the same as before the run.

The check of the result reads the output, not the main screen. Terra sends the result to `main/0` before its terminal owner restores the terminal (Terra `lib/terra/runtime.ex:282-288`, `356-359`), so the order of the two writes is a race. When `main/0` writes first, `{:ok, 1}` goes to the alternate screen and is not visible after the run. The "Code loading" row of [ERTS Differences](erts-differences.md) records this race (#428). The report gives the screen that shows the result after the run.

The checks were tested on 2026-10-11 with commands that break them on purpose. A command that runs the example and then sets `stty -echo` and writes `\e[?25l\e[1m\e[?1049h` failed the checks of the main screen, the cursor, the attributes, and `stty`. The command `sh -c 'echo "{:ok, 2}"'` failed the first step (`early_exit`) and the check of the result. The harness gave exit status 1 for both.

## Reference result

Measured on 2026-10-19 on `main` `ecb20086`. `mise run terra-reference` gave exit status 0, and each check of both runs passed. A second run gave the same inventory.

| Item | Value |
| --- | --- |
| Terra | `30dea5667d7e4ae5777aa3df77acb36c4aac6967`, `mix.lock` blob `76ff16d751c881b80e8c20cb15a347d22b5fb189` |
| ERTS | OTP 29, ERTS 17.1 (`erlang 29.1.1` of `mise.toml`) |
| Elixir | 1.20.4 (compiled with Erlang/OTP 29) |
| Rust | rustc 1.99.0 (not used by the ERTS run) |
| Host | macOS 27.0.1 (26A434), arm64 |
| Terminal | `script(1)` pseudo-terminal, 80 columns, 24 rows, `TERM=xterm-256color`, `LANG=LC_ALL=en_US.UTF-8` |
| First mode | `stty -g`: `gfmt1:cflag=4b00:iflag=2b02:lflag=5cb:oflag=3:...` (`icanon isig iexten echo`, `opost onlcr`, `-iutf8`) |

The steps showed `Count: 0` with `j/k to change, q to quit`, then the resize and the flush, then `Count: 1`, `Count: 2`, and `Count: 1`, in a box of 80 by 24 cells on the alternate screen. The resize step read the new size `30 100` and the raw-mode settings of the terminal at the resize (`stty-resize-size.txt`, `stty-resize.g`). After `q`, the command exited with status 0. The output ended with `\e[0m\e[?25h\e[?1049l{:ok, 1}\n`, so the main screen showed `{:ok, 1}` after the build messages. The cursor was visible, the attributes were reset, and `stty -g` after the run was the same as before the run.

In the inventory run, `main/0` wrote `{:ok, 1}` before the restore sequence, so the result went to the alternate screen. That run loads the modules before the run, as PBeam does, and the "Code loading" row of [ERTS Differences](erts-differences.md) gives this order for that case. Each other check passed.

Task #21 ran the same harness, without the resize and flush steps, on the PBeam runtime of `main` `ecb20086` on 2026-10-19 with `PBEAM_BIN=target/release/pbeam escript scripts/terra_reference.escript target/reference/pbeam -- bash scripts/example.sh terra_counter --no-build`, and got exit status 0. Each check of both runs passed: each step showed its screen, the exit statuses were 0, the output had `{:ok, 1}`, the main screen was active, the cursor was visible, the attributes were reset, and `stty -g` after equalled `stty -g` before. The two reports differ in the command line and in the banner lines of the example runner, and a step snapshot can show a partial frame; the final main screens, the terminal states, and the unknown-sequence lists are equal. #448 gives the PBeam run the start set of the build, so the `main` process waits for its module loads as in the reference run, and the restore sequence comes before the result. Task #832 added the resize and flush steps and ran the harness on PBeam again on 2026-10-19 with the same command. Both runtimes pass every step and every check. A step report holds the screen at the moment that its expected text is visible, so the run does not always finish drawing the frame first: the Start, Resize, and Flush screens can show a partial frame on either runtime, and the last runs showed it on ERTS. The final screens, the terminal states, the unknown-sequence lists, and the checks are equal; the reports differ in the command line, the runner banner lines, and these step snapshots. Terra keeps the cached 80 by 24 size on both runtimes, so neither view changes at the resize. The scenario still exercises none of the static candidates that the inventory lists as unexercised, and it does not give an error; those paths stay unverified.

The fixture disassembly of the entry, `mise run disasm target/examples/terra_counter/Elixir.TerraCounter.beam main/0`, gives `io:setopts/2` with `[binary]` on `standard_io`, an `is_eq_exact` test of `ok`, `'Elixir.Terra':run/1` with `'Elixir.Counter'`, and `'Elixir.IO':inspect/1` as the last call.

## Inventory

`target/reference/inventory.txt` gives the complete lists. This section gives the counts and the items that a PBeam run needs. Static candidates are the operations in the BEAM files that the example can call. Exercised operations are the operations that the call counters of the inventory run counted. A static candidate is not proof that the scenario runs the operation.

### Required modules

Static candidates: the BEAM list `target/examples/terra_counter/pbeam.beams` has each module that the example calls, directly or through other modules. `scripts/example_beams.exs` makes this list from the imports chunks of the example, `pbeam_boot`, and the boot modules. The pbeam run starts with a smaller start set (`target/examples/terra_counter/pbeam.start`), and loads each other module on demand through the pinned `error_handler` and `code_server`, as an interactive-mode ERTS run does (see "Code loading" in [ERTS Differences](erts-differences.md)). `target/examples/terra_counter/pbeam.unloaded` gives each called module that the PBeam loader does not load, with the reason: `Dict`, `HashDict`, and `HashSet` have no BEAM file, and the PBeam loader does not load the other modules.

Exercised: 78 modules have a called function. Each of them is in the BEAM list.

| Origin | Modules with a called function |
| --- | --- |
| `erts` (4) | `erlang`, `erts_internal`, `persistent_term`, `prim_file` |
| `kernel` (11) | `code`, `erl_signal_handler`, `file`, `file_server`, `group`, `net_kernel`, `os`, `prim_tty`, `prim_tty_sighandler`, `standard_error`, `user_drv` |
| `stdlib` (17) | `binary`, `filename`, `gen`, `gen_event`, `gen_server`, `gen_statem`, `io`, `io_lib`, `io_lib_format`, `lists`, `maps`, `proc_lib`, `proplists`, `queue`, `shell`, `unicode`, `unicode_util` |
| `elixir` (25) | `Access`, `Code`, `Code.Identifier`, `Enum`, `Enumerable.List`, `GenServer`, `IO`, `Inspect.Algebra`, `Inspect.Atom`, `Inspect.Integer`, `Inspect.Opts`, `Inspect.Tuple`, `Kernel`, `Keyword`, `List`, `Macro`, `Map`, `Port`, `Process`, `Range`, `String`, `String.Chars.Integer`, `String.Tokenizer`, `System`, `elixir_config` |
| `consolidated` (3) | `Enumerable`, `Inspect`, `String.Chars` |
| `terra` (16) | `Terra`, `Terra.Input`, `Terra.Input.Parser`, `Terra.Input.TTY`, `Terra.Renderer`, `Terra.Renderer.Grid`, `Terra.Renderer.Style`, `Terra.Runtime`, `Terra.Runtime.SignalHandler`, `Terra.Terminal`, `Terra.Terminal.ANSI`, `Terra.Terminal.Owner`, `Terra.Terminal.TTY`, `Terra.Theme`, `Terra.View`, `Terra.Width` |
| `example` (2) | `Counter`, `TerraCounter` |

### Host operations

The call counts give these host operations. The Terra and OTP lines give the arguments.

| Operation | Calls in the run | Source |
| --- | --- | --- |
| Port of `/bin/sh -c "stty -isig 2>/dev/null"` and `"stty isig 2>/dev/null"` with `[:binary, :exit_status, :nouse_stdio]`: `erts_internal:open_port/2` | 2 opens, 2 `erts_internal:port_info/1`, no `port_close` | Terra `lib/terra/terminal/tty.ex:88-121`. The port gives `{:exit_status, 0}` before `Port.info/1`, so `Port.close/1` does not run. |
| `os:find_executable("sh")`: `prim_file:read_info_nif/2` (stat of each `PATH` candidate) | 2 calls, 102 `read_info_nif/2` | Terra `lib/terra/terminal/tty.ex:89`. Each call reads the file information of `<directory>/sh` for each `PATH` directory, up to the first directory with an executable `sh` (`lib/kernel/src/os.erl:441-462` of OTP-29.1.1). In this run, that is 54 directories for each call. |
| Raw mode and cooked mode through `shell:start_interactive/1`: `prim_tty:tty_init/2` | 2 | Terra `lib/terra/terminal/tty.ex:69`, `79` |
| Terminfo values `co` and `xn`: `prim_tty:tigetnum_nif/1`, `prim_tty:tigetflag_nif/1` | 1 each | `prim_tty:init_term/1` of OTP |
| `prim_tty:isatty/1` | 10 | `prim_tty` of OTP |
| Window size (`ioctl(TIOCGWINSZ)`): `prim_tty:tty_window_size/1` | 5 | Terra `lib/terra/terminal/tty.ex:125-126` (`:io.columns/1`, `:io.rows/1` on `standard_error`) |
| Read of fd 0: `prim_tty:read_nif/3` | 5 (the run sends 4 keys and the partial escape sequence) | Terra `lib/terra/input/tty.ex:19` (`IO.binread(:stdio, 1)`) |
| Write to fd 1: `prim_tty:write_nif/2` | 7 | Terra `lib/terra/terminal/tty.ex:18` (`IO.binwrite/2`) and `IO.inspect/1` |
| `os:set_signal(:sigwinch, :handle)` | 1 | Terra `lib/terra/runtime.ex:315` |
| `os:getenv/1` | 2 | `os:find_executable/1` reads `PATH` (`lib/kernel/src/os.erl:413`) |

The boot of `erl -noshell` (for example `prim_tty:tty_create/1` and `setlocale/1`) runs before the counters start, so these counts do not have it. The boot modules of `pbeam_boot` are a separate requirement (see "Boot" in [ERTS Differences](erts-differences.md)). The scenario starts no other external program. It calls no network or file-write operation.

### BIFs and NIFs

Static candidates: 331 BIFs in external calls, 65 BIFs in `bif` and `gc_bif` instructions, and 139 NIF stubs (functions that call `erlang:nif_error/1,2` and are not BIFs).

Exercised: 60 BIFs and 8 NIFs. The called NIFs are `prim_file:read_info_nif/2` and the seven `prim_tty` NIFs of the host operations table. The called BIFs, with the count:

- `erlang`: `'++'/2` 152, `apply/2` 1, `atom_to_binary/2` 1, `atom_to_list/1` 8, `cancel_timer/1` 1, `demonitor/2` 148, `dt_restore_tag/1` 102, `dt_spread_tag/1` 102, `function_exported/3` 11, `group_leader/0` 19, `integer_to_binary/1` 94, `integer_to_binary/2` 1, `integer_to_list/1` 3, `iolist_to_binary/1` 40, `iolist_to_iovec/1` 16, `list_to_atom/1` 3, `list_to_binary/1` 7, `list_to_tuple/1` 102, `make_fun/3` 12, `make_ref/0` 4, `module_loaded/1` 2, `monitor/2` 143, `monitor/3` 11, `posixtime_to_universaltime/1` 6, `process_flag/2` 3, `process_info/2` 20, `put/2` 14, `raise/3` 6, `register/2` 1, `send/2` 12, `send/3` 7, `send_after/4` 2, `spawn_link/3` 1, `spawn_opt/4` 3, `split_binary/2` 1, `system_info/1` 211, `tuple_to_list/1` 1, `universaltime_to_localtime/1` 6, `whereis/1` 122.
- `erts_internal`: `map_next/3` 37, `open_port/2` 2, `port_info/1` 2, `trace_pattern/3` 2.
- Other modules: `binary:match/2` 2, `binary:split/3` 30, `file:native_name_encoding/0` 102, `lists:keyfind/3` 434, `lists:keymember/3` 2, `lists:member/2` 67, `lists:reverse/2` 252, `maps:merge/2` 1184, `maps:put/3` 1180, `net_kernel:dflag_unicode_io/1` 7, `os:getenv/1` 2, `os:set_signal/2` 1, `persistent_term:get/2` 2, `prim_file:internal_name2native/1` 102, `unicode:bin_is_7bit/1` 11, `unicode:characters_to_binary/2` 33, `unicode:characters_to_list/2` 1673.

Static candidates that the run does not call include `erlang:send_after/3` (the input flush timer calls `erlang:send_after/4`, and a cancel calls `erlang:cancel_timer/1`; Terra `lib/terra/input.ex:219`), `erts_internal:port_close/1`, the `prim_tty` NIFs `tty_select/2`, `tty_encoding/1`, `tigetstr_nif/1`, `tputs_nif/2`, `wcwidth/1`, and `wcswidth/1`, each `prim_file` NIF except `read_info_nif/2`, and each NIF of `prim_socket`, `prim_net`, `prim_buffer`, and `zlib`.

### Instructions

Static candidates: 122 instruction names in the BEAM list. The inventory gives a `test` instruction by the name of its test (for example `is_ne`), a `bs_match` instruction by each command (for example `bs_match:integer`), and a `bs_create_bin` instruction by each segment type (for example `bs_create_bin:append`).

Exercised: the called functions have 105 of these names. These are the names that no called function has: `bs_create_bin:float`, `bs_create_bin:utf16`, `bs_create_bin:utf32`, `bs_get_float2`, `bs_get_integer2`, `bs_get_utf16`, `bs_get_utf32`, `bs_match:skip`, `bs_match_string`, `bs_skip_bits2`, `bs_skip_utf16`, `bs_skip_utf32`, `bs_skip_utf8`, `fadd`, `fconv`, `fdiv`, `fmove`, `fmul`, `fnegate`, `fsub`, `is_number`, and `is_record`.

## Known uncertainty

- The scenario is one session with the keys `j j k q`, a resize, and an incomplete escape sequence. It does not give an error, so the code of the error path is static only. Terra keeps the terminal size that it caches at the start, so the terminal changes size and the following steps still pass, but no view changes; the harness does not show that the signal handler ran.
- The call counters count calls of functions, not instructions. A called function can have instructions that did not run, so the exercised instruction names are an upper bound.
- A BIF that the compiler makes a `bif` or `gc_bif` instruction, for example `erlang:element/2` in a guard, has no count. The 65 BIFs of these instructions are static candidates only.
- The counters start after the boot of `erl -noshell` and stop after the terminal owner of Terra stops. The count of `erts_internal:trace_pattern/3` comes from `terra_calls`, not from the scenario.
- The inventory run loads the modules of the BEAM list before the counters start, so its counts do not have the module loads of the code server. The reference run loads the modules on their first call. The order of the result and the restore sequence then changes (#428), but the other observable results are the same.
- The counts of `os:find_executable/1` depend on `PATH`. The `PATH` of `mise run` puts the mise tool directories first.
- The static candidates come from the imports chunks of the modules, not from a call graph. A function-level reachability scan with the PBeam loader is in the [#339 gap scan](https://github.com/fahchen/pbeam/issues/339#issuecomment-6099500139). That scan did not resolve 29 call sites with a module or function in a variable.
- The harness uses the macOS `script(1)` utility. It does not run on Linux.
- The `stty -g` check compares the mode after the command ends. ERTS sets fd 0 to its first mode at exit, so the check does not show if Terra restored `ISIG` itself. The two `stty` ports of the run show that Terra calls `stty -isig` and `stty isig`.
