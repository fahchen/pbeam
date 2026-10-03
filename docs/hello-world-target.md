# First target: PBeam prints Hello World

The first target is complete only when PBeam executes `Hello.main/0`, prints exactly
`Hello, world!\n` to stdout, and exits with status zero. Compilation, BEAM inspection,
and an ERTS reference run are preparation; none completes this target.

Terra Counter follows as a broader application compatibility target. The compiled ERTS
reference and both greeting variants are verified. The actual PBeam output path remains
open. Task #51 loads the complete pinned Elixir IO module. Tasks #48–#50 add only the
Elixir and OTP IO behavior reached by these fixtures. Task #42 then checks both entries
through that path. This does not claim general Elixir IO or broad OTP support.

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
1205/1205 (100%). In-process tests measure fixture coverage; subprocess tests
independently verify the original compiled artifacts and process-level results.

| Kind | Instruction or operation | Required semantics | Evidence | PBeam support and limits | Task / PR / test |
| --- | --- | --- | --- | --- | --- |
| Instruction | `move literal, x0` | Put the binary literal into argument register 0 | Static entry disassembly; PBeam verification remains pending | Reads uncompressed OTP 29 literal tables and ETF binary values; other literal values remain unsupported | #42; `hello.rs` |
| Instructions | `call`, `call_ext`, `call_ext_last`, `allocate`, `deallocate`, `move x/y` | Preserve arguments and return values across a local call and startup-loaded external calls | Compiled Erlang call fixture, compared with ERTS | Supports this bounded call fixture only; no scheduler, exceptions, or dynamic loading | #5; `calls.rs` |
| Instruction | `call_ext_only 1, Elixir.IO:puts/1` | External tail call with one argument | Static entry instruction; PBeam verification remains pending | Supports this `call_ext_only` form for this fixture | #42; `hello.rs` |
| Loader metadata | `label`, `line`, `func_info` | Resolve entry label 11 and skip line metadata | Static `main/0` disassembly; PBeam verification remains pending | Skips line metadata; generated `select_val`, `call_only`, and `jump` stay unsupported at execution | #42; `hello.rs` |
| Terms | Binary literals, atom `ok` | Load each greeting and return the `:io.put_chars/2` result | Static entry disassembly; PBeam verification remains pending | Supports referenced UTF-8 ETF binaries and required atoms only as verified by the task chain | #42; `hello.rs` |
| Module loading | Complete `Elixir.IO.beam` code chunk | Read all code and retain unsupported instructions | `loads_the_pinned_elixir_io_module` with Elixir 1.20.4 and OTP 29.1.1 | Decode forms used by this module; do not execute unsupported forms | #51; PR #47 |
| External MFA | `Elixir.IO:puts/1` | Execute the compiled Elixir implementation | Pinned Elixir `Elixir.IO.beam` disassembly | No native mapping is accepted for this MFA | #48 |
| External MFA | `Elixir.IO:puts/2` | Map `:stdio`, convert binary to chardata, append newline, call `:io.put_chars/2` | Compiled BEAM disassembly and Elixir `io.ex` source | Implement only the fixture's binary standard-output path | #48 |
| Process and IO protocol | `:io.put_chars/2` | Send an IO request to the group leader and receive its matching reply | OTP 29 `io.beam` disassembly and `stdlib/src/io.erl` | Implement only the request, reply, monitor, and process behavior reached by this call | #49 |
| Host operation | Group-leader output | Handle the standard-output request, write bytes, and return `io_reply` | OTP 29 `io.erl` request protocol | Host stdout is a group-leader endpoint; it does not replace Elixir or `:io` BEAM functions | #50 |
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
monitor. The group leader writes to host stdout.

Tasks #51, #48, #49, and #50 implement this observed path in small steps. They must execute the
compiled Elixir and OTP BEAM functions. They must not intercept
`Elixir.IO:puts/1` or `:io:put_chars/2`, print the greeting directly, or alter
the fixture to use a PBeam-only operation. The support limit is the binary
standard-output path reached by these fixtures. General chardata, custom devices,
and unrelated process behavior remain out of scope.

After the task chain is implemented, compile the Elixir fixture from
`fixtures/hello`, then run PBeam from the repository root. PR #47 removes the direct
IO shortcut. The command still cannot complete the greeting because task #48 must
add execution for the compiled `Elixir.IO` path:

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
| 6 | [#49: Execute OTP IO protocol](https://github.com/fahchen/pbeam/issues/49) | Run the `:io` request/reply path through the group leader |
| 7 | [#50: Write group-leader output](https://github.com/fahchen/pbeam/issues/50) | Write the standard-output request and return its reply |
| 8 | [#42: PBeam printing](https://github.com/fahchen/pbeam/issues/42) | Both greeting variants, stdout/stderr/exit checks, and visible failures |

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
task. PR #47 implements task #51 and is ready for review. It loads the pinned
`Elixir.IO.beam` module and retains unsupported instructions. The `puts` path remains
unsupported until task #48.

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
