# First target: PBeam prints Hello World

The first target is complete only when PBeam executes `Hello.main/0`, prints exactly
`Hello, world!\n` to stdout, and exits with status zero. Compilation, BEAM inspection,
and an ERTS reference run are preparation; none completes this target.

Terra Counter follows as a broader application compatibility target. The compiled ERTS
reference and both greeting variants are verified. Task #42 adds a small PBeam command
that runs these two entries. General Elixir IO and broad OTP support remain unsupported.

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
1097/1097 (100%). In-process tests measure fixture coverage; subprocess tests
independently verify the original compiled artifacts and process-level results.

| Kind | Instruction or operation | Required semantics | Evidence | PBeam support and limits | Task / PR / test |
| --- | --- | --- | --- | --- | --- |
| Instruction | `move literal, x0` | Put the binary literal into argument register 0 | Static entry disassembly and both PBeam runs | Reads uncompressed OTP 29 literal tables and ETF binary values; other literal values remain unsupported | #42; `hello.rs` |
| Instructions | `call`, `call_ext`, `call_ext_last`, `allocate`, `deallocate`, `move x/y` | Preserve arguments and return values across a local call and startup-loaded external calls | Compiled Erlang call fixture, compared with ERTS | Supports this bounded call fixture only; no scheduler, exceptions, or dynamic loading | #5; `calls.rs` |
| Instruction | `call_ext_only 1, Elixir.IO:puts/1` | External tail call with one argument | Static entry instruction and both PBeam runs | Supports this `call_ext_only` form and the two required calls | #42; `hello.rs` |
| Loader metadata | `label`, `line`, `func_info` | Resolve entry label 11 and skip line metadata | Static `main/0` disassembly and successful PBeam runs | Skips line metadata; generated `select_val`, `call_only`, and `jump` stay unsupported at execution | #42; `hello.rs` |
| Terms | Binary literals, atom `ok` | Load each greeting and return `:ok` from the output call | Both PBeam runs and output unit tests | Supports referenced UTF-8 ETF binaries and the required atom; other referenced literal types fail | #42; `hello.rs` |
| External MFA | `Elixir.IO:puts/1` | Write supplied greeting plus newline to standard output | ERTS reference and PBeam subprocess tests | Accepts one UTF-8 binary; rejects other arguments and arities | #42; `hello.rs` |
| Host operation | stdout write | Write exact bytes and show write failures | PBeam output tests with a failing writer | Uses Rust `Write`; no device lookup or general IO requests | #42; `program.rs` tests |
| Static library dependency | `IO.puts/2`, `:io.put_chars/2` | Device mapping, chardata conversion, IO request handling | Installed Elixir `lib/elixir/lib/io.ex`, `puts` definition; not independently traced | Not required for the two binary fixtures | #42 |
| Compiler-generated functions | `__info__/1`, `module_info/0,1`, helper; `:erlang.get_module_info/1,2` imports | Load the whole module without running unused generated code | Static exports/imports and successful entry runs | Retains the observed `select_val`, `call_only`, `jump`, and unsupported literal forms; execution fails if reached | #42; `hello.rs` |

Entry disassembly contains a literal move followed by the external tail call;
there is no stdout opcode. Static inspection is not an opcode execution trace,
and successful output does not establish the complete dynamic OTP dependency
set. The table records the bounded entry path and observed behavior, not general
OTP support.

### Implemented output boundary

For #42, resolve the ordinary external MFA `Elixir.IO:puts/1` to a minimal Rust
implementation accepting the fixture's valid UTF-8 binary, appending one newline,
writing to stdout, and returning `:ok`. Validate the supported argument form and
propagate write failures; do not recognize greeting contents or delegate to ERTS.
Keep both literal variants as regression tests.

Executing the full Elixir/OTP IO implementation would introduce device resolution,
chardata conversion, and process-based IO behavior before this checkpoint. The
narrow boundary preserves this fixture's source and observable behavior while
explicitly leaving arbitrary chardata, custom devices, and general IO unsupported.
This supports the two compiled fixtures. It does not support arbitrary chardata,
custom devices, or general IO. The implementation uses the existing external-MFA
path from #5. It does not add the native registry in #12. Task #42 adds no other
native mapping.

Run the Elixir compile step from `fixtures/hello`, then run PBeam from the repository
root:

```sh
mix compile --warnings-as-errors
cargo run --bin pbeam -- Elixir.Hello main fixtures/hello/_build/dev/lib/hello_reference/ebin/Elixir.Hello.beam
```

Pass another module name and BEAM file to run the second fixture. Pass more BEAM
files after the entry name and function to load modules for external calls.

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
| 4 | [#42: PBeam printing](https://github.com/fahchen/pbeam/issues/42) | Both greeting variants, stdout/stderr/exit checks, and visible failures |

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
its stdout, stderr, and exit status. Task #42 implements the command and tests both
compiled greeting modules. Its PR remains under review until merge.

[Reference task #41](https://github.com/fahchen/pbeam/issues/41) captures the ERTS
baseline and disassembles its compiler output as preparation. Existing constant-return
and call-chain tasks remain small interpreter building blocks. These tasks may be
reviewed independently, but the Hello World target remains open until
[execution task #42](https://github.com/fahchen/pbeam/issues/42) demonstrates printing
through PBeam and its regression test passes.
The Hello World execution task adds only the output path and any proven prerequisites
still required by that baseline; split additional behaviors into reviewed task issues
if needed. Preserve the language-visible call semantics and record any native/host
implementation decision in its PR. Do not bypass the gate by changing the program to
a PBeam-only print operation.

Lint and 100% application line coverage remain required. Adding the Elixir fixture
uses formatting, warnings-as-errors, and Mix tests with a 100% coverage threshold
in the shared check command. Passing
Hello World establishes this program's behavior, not general Elixir/OTP support.

After this checkpoint, continue the planned runtime, native, and packaging slices.
Then capture and implement the [Terra Counter contract](terra-target.md), retaining
the Hello World test as a regression check.
