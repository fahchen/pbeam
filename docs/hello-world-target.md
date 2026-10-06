# First target: PBeam prints Hello World

The first target is complete only when PBeam executes `Hello.main/0`, prints exactly
`Hello, world!\n` to stdout, and exits with status zero. Compilation, BEAM inspection,
and an ERTS reference run are preparation; none completes this target.

Terra Counter follows as a broader application compatibility target. The compiled ERTS
reference and both greeting variants are verified. PR #52 executes the pinned compiled
Elixir IO path through `:io.put_chars/2`. Tasks #49, #55, #50, and #79 add only the process runtime, OTP execution, native binding, and host behavior reached by these fixtures. Task #42 checks both entries through that path with the `pbeam` binary.
This does not claim general Elixir IO or broad OTP support.

## Program

PBeam must execute this ordinary Elixir program. Use the pinned mise toolchain to
compile it as test setup:

```elixir
defmodule Hello do
  def main do
    IO.puts("Hello, world!")
  end
end
```

The entry MFA is `Elixir.Hello:main/0`. The reference task must record the exact
compile/run commands, tool versions, emitted BEAM modules, and operations actually
required by `IO.puts/1`. Do not assume console output is a single BEAM instruction.

## Instruction and operation inventory

The reference uses OTP 29.1.1 and Elixir 1.20.4-otp-29 on macOS arm64.
From a clean checkout, install the pinned tools with `mise install`, then run:

```sh
mise run reference
mise run inventory
```

`reference` compiles the fixture in Mix's test environment, covers both entry
functions, and runs each compiled artifact in a fresh ERTS process. Tests compare
stdout, stderr, and exit status exactly. Each child installs a five-second execution
watchdog (exit 124); ExUnit's default 60-second test timeout bounds the surrounding
test. `inventory` compiles in the dev environment and prints entry disassembly,
exports, and imports using `:beam_disasm` and `:beam_lib`. These inspection APIs
are tooling for the pinned OTP release, not a PBeam runtime dependency.

Artifacts are `Elixir.Hello.beam` and `Elixir.HelloVariant.beam` under
`fixtures/hello/_build/{test,dev}/lib/hello_reference/ebin/`. Generated BEAM and
coverage files are ignored and rebuilt from the checked-in source. Mix also
emits `hello_reference.app`; the fixture has no third-party dependencies.

Observed results: four tests pass; each module has 100% application line coverage.
Both subprocesses have empty stderr and exit zero. Stdout is `Hello, world!\n`
and `Another greeting.\n`, respectively. Rust application line coverage is
1434/1434 (100%). In-process tests measure fixture coverage; subprocess tests
independently verify the original compiled artifacts and process-level results.

| Kind | Instruction or operation | Required semantics | Evidence | PBeam support and limits | Task / PR / test |
| --- | --- | --- | --- | --- | --- |
| Instruction | `move literal, x0` | Put the binary literal into argument register 0 | `executes_compiled_elixir_puts_to_the_otp_io_boundary`; PR #52 | Reads the referenced OTP 29 binary literals used by these fixtures; other literal values remain unsupported | #48; `hello.rs` |
| Instructions | `call`, `call_ext`, `call_ext_last`, `allocate`, `deallocate`, `move x/y` | Preserve arguments and return values across a local call and startup-loaded external calls | Compiled Erlang call fixture, compared with ERTS | Supports this bounded call fixture only; no scheduler, exceptions, or dynamic loading | #5; `calls.rs` |
| Instruction | `call_ext_only 1, Elixir.IO:puts/1` | External tail call with one argument | `executes_compiled_elixir_puts_to_the_otp_io_boundary`; PR #52 | Executes the loaded Elixir implementation and reaches `io:put_chars/2` | #48; `hello.rs` |
| Loader metadata | `label`, `line`, `func_info` | Resolve the entry label and skip line metadata | Compiled `Hello.main/0` and `Elixir.IO` execution; PR #52 | Executes the forms reached by this path, including `select_val`, `call_only`, and `jump`; other generated functions remain untested | #48; `hello.rs` |
| Terms | Binary literals, chardata list, atom `ok` | Pass greeting chardata to `io:put_chars/2` and return the target result | A test-only BEAM probe verifies the call and return; PR #52. The booted `pbeam` runs of both entries are compared with ERTS | `io:put_chars/2` returns `ok` through the OTP request and reply path, and `boot_probe` matches this result. The entry returns it to `pbeam_boot`, and the run exits with status 0 | #48; #42; `hello.rs`, `boot.rs` |
| Module loading | Complete `Elixir.IO.beam` code chunk | Read all code and retain unsupported instructions | `loads_the_pinned_elixir_io_module` with Elixir 1.20.4 and OTP 29.1.1 | Decode forms used by this module; do not execute unsupported forms | #51; PR #47 |
| External MFA | `Elixir.IO:puts/1` | Execute the compiled Elixir implementation | Pinned Elixir BEAM execution; PR #52 | No native mapping is used | #48; `hello.rs` |
| External MFA | `Elixir.IO:puts/2` | Map `:stdio`, convert binary to chardata, append newline, call `:io.put_chars/2` | Compiled BEAM execution and test-only BEAM probe; PR #52. `prints_each_greeting_through_the_booted_otp_io_servers` compared with ERTS | Runs the binary standard-output path of both fixtures through the OTP IO servers to the host write. Other devices and chardata forms are not verified | #48; #42; `hello.rs` |
| Process and IO protocol | `:io.put_chars/2` through `user`, `user_drv`, and the `prim_tty` writer | Interpret the pinned BEAM path through the `prim_tty:write_nif/2` boundary; support the required process and IO request/reply behavior | OTP 29.1.1 source (io.erl:265-266, 1652-1655; init.erl:1539-1546; prim_tty.erl:555-566); `executes_the_pinned_otp_io_request_path_to_the_writer_nif`; the booted `pbeam` runs of both entries compared with ERTS | `pbeam_boot` starts the servers and sets `user` as the group leader of the entry. The request and reply path writes each greeting. When the host write fails, `user_drv` stops and `io:put_chars/2` raises `error:terminated` in the entry (io.erl:197-202, 782, 1679-1684). Only the instructions, scheduling, process, mailbox, and monitor behavior reached by this path and the boot are implemented; see [ERTS Differences](erts-differences.md) | #49; #55; #79; #158; #42; `io_path.rs`, `io_fail.rs`, `hello.rs` |
| Native binding and host operation | `prim_tty:write_nif/2` | Dispatch this exact verified OTP MFA to a PBeam native binding at function entry; write supplied bytes to the host stdout of a TTY resource and return success or failure to the BEAM path | OTP 29.1.1 `prim_tty` NIF declaration, load path, and terminal path (prim_tty_nif.c:354-427); `prim_tty_probe:main/0` and `host_write_probe:main/0` compared with ERTS; the booted `pbeam` runs of both entries | Statically linked NIF binding at `nif_start`. For a TTY resource, it writes the iovec to the host fd with `writev` and returns `ok` or `{error, Errno}`, as ERTS does. For a term that is not a TTY resource, it raises `error:badarg`, as ERTS does. This does not establish support for other OTP NIFs or replace BEAM IO functions | #50; #134; #42; `prim_tty.rs`, `io_path.rs`, `hello.rs` |
| Compiler-generated functions | `__info__/1`, `module_info/0,1`, helper; `:erlang.get_module_info/1,2` imports | Load the whole module without running unused generated code | Static exports/imports; the booted `pbeam` runs of both entries load the whole module | Loaded, not run on this path. Retains the observed `select_val`, `call_only`, `jump`, and unsupported literal forms; execution fails if reached | #42; `hello.rs` |
| Command | `pbeam pbeam_boot start <Module> main -- <BEAM files>` | Load the entry module, the pinned `Elixir.IO` module, the 41 pinned boot modules, and `examples/hello_world/pbeam_boot.beam`; boot the standard IO servers and run the entry | `prints_each_greeting_through_the_booted_otp_io_servers` compared with `erl -noshell -pa <fixture ebin> -pa <Elixir ebin> -eval "'<Module>':main(), halt()."` with `TERM=dumb`, `LC_ALL=C.UTF-8`, and stdin `/dev/null` | `Elixir.Hello` gives exactly `Hello, world!\n` and `Elixir.HelloVariant` gives exactly `Another greeting.\n` on stdout, with empty stderr and status 0, as ERTS. The test runs the `pbeam` binary without `PATH`, so it cannot start `erl` or `elixir`. Only a locale whose codeset is UTF-8 is supported | #42; `hello.rs` |
| Failures | Missing function export, entry module that the run does not load, `LC_ALL=C`, and stdout a pipe whose read end is closed | Fail visibly with status 1; match ERTS or record the difference | `fails_visibly_for_a_missing_entry_through_the_boot`, `stops_at_the_binary_match_of_a_locale_that_is_not_utf8`, and `raises_terminated_in_the_entry_after_a_failed_stdout_write` compared with ERTS | `Elixir.Hello start` raises `undef` with the ERTS first frame `{'Elixir.Hello',start,[],[]}`. `Elixir.String.Chars main` stops with `unresolved external MFA` (row "Undefined functions"). `LC_ALL=C` stops with `unsupported bs_start_match3 of a binary: needs a match context`, but ERTS writes the greeting (rows "Boot" and "Binary matching"). A failed stdout write gives the root exit `terminated`, as ERTS. Each case exits with status 1. PBeam writes `pbeam: root process exited with <reason>` with the stack trace `[]`, and ERTS writes its init halt text (rows "Exception stack trace" and "Run end"; task #6) | #42; `hello.rs` |

Entry disassembly contains a literal move followed by the external tail call;
there is no stdout opcode. Static inspection is not an opcode execution trace,
and successful output does not establish the complete dynamic OTP dependency
set. The table records the bounded entry path and observed behavior, not general
OTP support.

### Observed IO path

The compiled entry calls `Elixir.IO:puts/1`. The pinned Elixir BEAM code calls
`Elixir.IO:puts/2`, builds chardata with a newline, and calls
`:io:put_chars/2`. OTP maps `:stdio` to `:standard_io`, obtains the group leader,
and sends an `io_request`. It waits for the matching `io_reply` and removes its
monitor. The group leader forwards the request to `user_drv`. The pinned OTP
29.1.1 terminal path reaches the `prim_tty:write_nif/2` NIF for host output.
`io.erl` matches the reply reference or a matching `DOWN` message. Task #49 is
scoped to its zero-time `EXIT` cleanup and preservation of unmatched messages.
It does not add general selective receive. `call_ext*` means an imported call, not a call outside the BEAM VM. Interpret
each available BEAM implementation, including the Elixir and OTP modules. Add
only the runtime behavior and native host operation that the traced path needs.

Task #49 adds the cooperative process runtime for this path. Task #55 interprets the OTP BEAM path through `prim_tty:write_nif/2`. Task #50 registers that exact native MFA and runs its binding at function entry. Task #79 adds the TTY resource and the host write of the binding (#134), and the boot module `pbeam_boot` starts the OTP IO servers (#139). Tasks #51, #48, #49, #55, #50, and #79 implement this observed path in small steps. Task #42 checks both entries through this path with the `pbeam` binary. These tasks execute the compiled Elixir and OTP BEAM functions. They do not intercept `Elixir.IO:puts/1` or `:io:put_chars/2`, print the greeting directly, or alter the fixture to use a PBeam-only operation. The support limit is the binary standard-output path reached by these fixtures. General chardata, custom devices, and unrelated process behavior remain out of scope.

To run Hello World, compile the Elixir fixture from `fixtures/hello`, then run PBeam from the repository root. PR #47 removed the direct IO shortcut. PR #52 executes compiled Elixir IO through the OTP call boundary. Tasks #55, #50, and #79 execute the OTP path through the host boundary. Task #42 adds the regression tests of the command in `tests/hello.rs`.

The `pbeam` command loads the given BEAM files and runs one function as the root process. It does not start the OTP standard IO servers. The boot module `pbeam_boot` in [`examples/hello_world`](../examples/hello_world/README.md) starts them (#139). `pbeam_boot:start(Module, Function)` then runs the entry with the registered `user` process as its group leader. The boot calls 41 pinned OTP modules: `erlang`, `prim_tty`, `proc_lib`, `application`, `application_controller`, `lists`, `os`, `unicode`, `gen_event`, `gen`, `prim_tty_sighandler`, `erl_signal_handler`, `standard_error`, `supervisor_bridge`, `user_sup`, `user_drv`, `group`, `gen_statem`, `io`, `io_lib`, `io_lib_format`, `proplists`, `queue`, `gen_server`, `logger`, `logger_config`, `edlin`, `edlin_key`, `logger_server`, `logger_simple_h`, `logger_sup`, `supervisor`, `logger_std_h`, `logger_h_common`, `logger_olp`, `logger_proxy`, `logger_handler_watcher`, `logger_formatter`, `maps`, `init`, and `erts_internal`. Pass their BEAM files and `examples/hello_world/pbeam_boot.beam` after `--`. Without one of them, the command stops with `unresolved external MFA` of that module and status 1. Run the command in a locale whose codeset is UTF-8, such as `LC_ALL=C.UTF-8`. In another locale, the command stops with `unsupported bs_start_match3 of a binary: needs a match context` (see "Boot" in [ERTS Differences](erts-differences.md)).

From the repository root, these commands compile the Elixir fixture and run `pbeam_boot:start('Elixir.Hello', main)` with `Elixir.IO.beam` of the pinned Elixir and the boot modules. The run prints `Hello, world!\n` and exits with status zero:

```sh
(cd fixtures/hello && mise exec -- mix compile --warnings-as-errors)
boot_modules="erlang prim_tty proc_lib application application_controller lists os unicode gen_event gen prim_tty_sighandler erl_signal_handler standard_error supervisor_bridge user_sup user_drv group gen_statem io io_lib io_lib_format proplists queue gen_server logger logger_config edlin edlin_key logger_server logger_simple_h logger_sup supervisor logger_std_h logger_h_common logger_olp logger_proxy logger_handler_watcher logger_formatter maps init erts_internal"
boot_beams=$(mise exec -- erl -noshell -eval '[io:format("~ts~n", [case code:which(list_to_atom(M)) of preloaded -> filename:join([code:lib_dir(erts), "ebin", M ++ ".beam"]); Path -> Path end]) || M <- init:get_plain_arguments()], halt().' -extra $boot_modules)
elixir_io=$(mise exec -- elixir -e 'IO.puts(:code.which(IO))')
LC_ALL=C.UTF-8 cargo run --bin pbeam -- pbeam_boot start Elixir.Hello main -- fixtures/hello/_build/dev/lib/hello_reference/ebin/Elixir.Hello.beam $elixir_io $boot_beams examples/hello_world/pbeam_boot.beam
```

Pass another module name and BEAM file to run the second fixture. Pass more BEAM files after `--` to load modules for external calls. Run `cargo run --bin pbeam -- --help` to print the command options.

With `boot_beams` from the commands above, this command runs the boot fixture. It also prints `Hello, world!\n` and exits with status zero:

```sh
LC_ALL=C.UTF-8 cargo run --bin pbeam -- pbeam_boot start boot_probe main -- fixtures/boot/boot_probe.beam $boot_beams examples/hello_world/pbeam_boot.beam
```

Each implementation PR updates affected rows with supported semantics, remaining
limits, and runnable regression tests. Mark support verified only after merge and
passing acceptance checks. GitHub links below identify the tasks; PR #43 supplies
the reference evidence and `fixtures/hello/test/hello_test.exs` contains its checks.

## Implementation checklist

Rebuild the call fixture with the pinned Erlang compiler:

```sh
mise exec -- erlc +no_line_info -Werror -o fixtures/calls fixtures/calls/calls.erl fixtures/calls/external.erl
```

The checked-in BEAM files are the test inputs. The Rust call test also runs the
same modules under ERTS and checks its output.

Keep implementation todos as checkboxes in the linked GitHub task issues; this
document records technical evidence and verified support. Each todo names one
observable behavior and its acceptance check. Group related inventory rows into
a focused task rather than creating one issue per opcode. Additional independent
behaviors become sibling issues under the epic, with native dependencies.

| Order | Task | Result required for review |
| --- | --- | --- |
| 1 | [#41: Reference and inventory](https://github.com/fahchen/pbeam/issues/41) | Compiled fixtures, exact ERTS output checks, inventory, and proposed output boundary |
| 2 | [#4: Constant return](https://github.com/fahchen/pbeam/issues/4) | Real BEAM loading and execution, plus malformed-input checks |
| 3 | [#5: Calls](https://github.com/fahchen/pbeam/issues/5) | Local/external calls, preserved arguments, and correct returns |
| 4 | [#51: Load Elixir IO](https://github.com/fahchen/pbeam/issues/51) | Load the pinned module and retain unsupported instructions |
| 5 | [#48: Execute Elixir IO](https://github.com/fahchen/pbeam/issues/48) | Run compiled `Elixir.IO` code for the fixture's binary path |
| 6 | [#49: Add a cooperative process runtime](https://github.com/fahchen/pbeam/issues/49) | Process, mailbox, reply-matching, and monitor behavior on one cooperative scheduler thread for the IO request path |
| 7 | [#55: Execute OTP group-leader IO](https://github.com/fahchen/pbeam/issues/55) | Interpret `:io`, `group`, `user_drv`, and `prim_tty` BEAM code through `prim_tty:write_nif/2` |
| 8 | [#50: Write standard IO through the group leader](https://github.com/fahchen/pbeam/issues/50) | Register `prim_tty:write_nif/2` as a native binding and run it at function entry |
| 9 | [#79: Start the OTP standard IO servers](https://github.com/fahchen/pbeam/issues/79) | Start the IO servers through OTP code, and add the TTY resource and the host write of `prim_tty:write_nif/2` |
| 10 | [#42: PBeam printing](https://github.com/fahchen/pbeam/issues/42) | Both greeting variants, stdout/stderr/exit checks, and visible failures |

Update the inventory, issue checklists, dependencies, and PR decisions together
when evidence changes the chosen solution. Implementation progress and readiness
remain in GitHub; the table above describes delivery boundaries.

## Acceptance

- On macOS arm64, PBeam running the compiled entry produces exactly `Hello, world!\n`
  on stdout, no application diagnostics on stderr, and exit status zero.
- The process terminates without input, a PTY, network access, or credentials.
- The same captured output and exit behavior hold on ERTS and PBeam.
- PBeam reads the compiled artifact and executes the entry. A Rust launcher that
  prints the expected text without executing BEAM does not satisfy this target.
- A second compiled fixture with a different literal must produce that literal,
  demonstrating that the output comes from the program rather than a special case.
- Missing entry functions and output failures must fail visibly; test their
  supported error behavior rather than treating them as successful runs.

Initially, a development runner may receive loose BEAM files. A standalone embedded
executable remains the later packaging milestone. The PBeam run itself must not
delegate execution to ERTS or an external Elixir process.

## Delivery boundary

The deliverable is a runnable PBeam command plus a regression test that captures
its stdout, stderr, and exit status. Task #42 is the final command and acceptance
task. PR #47 implements task #51 and merged as `265f173`. It loads the pinned
`Elixir.IO.beam` module. PR #52 implements task #48 and executes compiled `puts/1`
and `puts/2` through the `:io.put_chars/2` boundary. Task #55 executes the OTP IO path to `prim_tty:write_nif/2`. Task #50 runs the native binding of `prim_tty:write_nif/2`. Task #79 adds the TTY resource and the host write, and `pbeam_boot` starts the OTP IO servers. Task #42 runs both entries with the `pbeam` binary and checks their stdout, stderr, and exit status against ERTS, also for the failures.

[Reference task #41](https://github.com/fahchen/pbeam/issues/41) captures the ERTS
baseline and disassembles its compiler output as preparation. Existing constant-return
and call-chain tasks remain small interpreter building blocks. Review tasks #51 and #48–#50
independently. Keep the Hello World target open until [task #42](https://github.com/fahchen/pbeam/issues/42)
executes the compiled path and its regression checks pass. Do not bypass the gate by
changing the program to a PBeam-only print operation.

Lint and 100% application line coverage remain required. Adding the Elixir fixture
uses formatting, warnings-as-errors, and Mix tests with a 100% coverage threshold
in the shared check command. Passing
Hello World establishes this program's behavior, not general Elixir/OTP support.

After this checkpoint, continue the planned runtime, native, and packaging slices.
Then capture and implement the [Terra Counter contract](terra-target.md), retaining
the Hello World test as a regression check.
