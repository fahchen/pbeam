# First target: PBeam prints Hello World

The first target is complete only when PBeam executes `Hello.main/0`, prints exactly
`Hello, world!\n` to stdout, and exits with status zero. Compilation, BEAM inspection,
and an ERTS reference run are preparation; none completes this target.

Terra Counter follows as a broader application compatibility target. The compiled ERTS
reference and both greeting variants are verified. PR #52 executes the pinned compiled
Elixir IO path through `:io.put_chars/2`. Tasks #49, #50, and #79 add only the OTP, native binding, and host behavior reached by these fixtures. Task #42 then checks both entries through that path.
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
| Terms | Binary literals, chardata list, atom `ok` | Pass greeting chardata to `io:put_chars/2` and return the target result | A test-only BEAM probe verifies the call and return; PR #52 | The probe returns the chardata. The actual OTP result remains pending task #49 | #48; `hello.rs` |
| Module loading | Complete `Elixir.IO.beam` code chunk | Read all code and retain unsupported instructions | `loads_the_pinned_elixir_io_module` with Elixir 1.20.4 and OTP 29.1.1 | Decode forms used by this module; do not execute unsupported forms | #51; PR #47 |
| External MFA | `Elixir.IO:puts/1` | Execute the compiled Elixir implementation | Pinned Elixir BEAM execution; PR #52 | No native mapping is used | #48; `hello.rs` |
| External MFA | `Elixir.IO:puts/2` | Map `:stdio`, convert binary to chardata, append newline, call `:io.put_chars/2` | Compiled BEAM execution and test-only BEAM probe; PR #52 | Supports the fixture's binary standard-output path; actual OTP IO remains pending | #48; `hello.rs` |
| Process and IO protocol | `:io.put_chars/2` through `group`, `user_drv`, and `prim_tty` | Interpret the pinned BEAM path through the `prim_tty:write_nif/2` boundary; support the required process and IO request/reply behavior | OTP 29.1.1 disassembly and source | Implement only the instructions, scheduling, process, mailbox, and monitor behavior reached by this path | #49 |
| Native binding and host operation | `prim_tty:write_nif/2` | Dispatch this exact verified OTP MFA to a PBeam native binding at function entry; write supplied bytes to the host stdout of a TTY resource and return success or failure to the BEAM path | OTP 29.1.1 `prim_tty` NIF declaration, load path, and terminal path; `prim_tty_probe:main/0` compared with ERTS | First statically linked NIF binding. Task #50 runs it at `nif_start`. PBeam has no TTY resource, so the binding raises `error:badarg`, as ERTS does for a term that is not a TTY resource. Task #79 adds the TTY resource and the host write. This does not establish support for other OTP NIFs or replace BEAM IO functions | #50; `prim_tty.rs`, `io_path.rs`; #79 |
| Compiler-generated functions | `__info__/1`, `module_info/0,1`, helper; `:erlang.get_module_info/1,2` imports | Load the whole module without running unused generated code | Static exports/imports; PBeam verification remains pending | Retains the observed `select_val`, `call_only`, `jump`, and unsupported literal forms; execution fails if reached | #42; `hello.rs` |

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

Task #49 interprets the OTP BEAM path through `prim_tty:write_nif/2`. Task #50 registers that exact native MFA and runs its binding at function entry. Task #79 starts the OTP IO servers, adds the TTY resource, and implements the host write of the binding. Tasks #51, #48, #49, #50, and #79 implement this observed path in small steps. They must execute the compiled Elixir and OTP BEAM functions. They must not intercept `Elixir.IO:puts/1` or `:io:put_chars/2`, print the greeting directly, or alter the fixture to use a PBeam-only operation. The support limit is the binary standard-output path reached by these fixtures. General chardata, custom devices, and unrelated process behavior remain out of scope.

After the task chain is implemented, compile the Elixir fixture from `fixtures/hello`, then run PBeam from the repository root. PR #47 removed the direct IO shortcut. PR #52 executes compiled Elixir IO through the OTP call boundary. The command cannot complete the greeting until tasks #50 and #79 execute the OTP path through the host boundary and task #42 verifies the command:

```sh
mix compile --warnings-as-errors
cargo run --bin pbeam -- Elixir.Hello main fixtures/hello/_build/dev/lib/hello_reference/ebin/Elixir.Hello.beam
```

Pass another module name and BEAM file to run the second fixture. Pass more BEAM
files after the entry name and function to load modules for external calls.
Run `cargo run --bin pbeam -- --help` to print the command options.

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
| 6 | [#49: Execute OTP IO protocol](https://github.com/fahchen/pbeam/issues/49) | Interpret `:io`, `group`, `user_drv`, and `prim_tty` BEAM code through `prim_tty:write_nif/2` |
| 7 | [#50: Write standard IO through the group leader](https://github.com/fahchen/pbeam/issues/50) | Register `prim_tty:write_nif/2` as a native binding and run it at function entry |
| 8 | [#79: Start the OTP standard IO servers](https://github.com/fahchen/pbeam/issues/79) | Start the IO servers through OTP code, and add the TTY resource and the host write of `prim_tty:write_nif/2` |
| 9 | [#42: PBeam printing](https://github.com/fahchen/pbeam/issues/42) | Both greeting variants, stdout/stderr/exit checks, and visible failures |

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
and `puts/2` through the `:io.put_chars/2` boundary. Task #50 runs the native binding of `prim_tty:write_nif/2`. The start of the OTP IO servers, the TTY resource, and the host write remain for task #79.

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
