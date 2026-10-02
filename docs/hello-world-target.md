# First target: PBeam prints Hello World

The first target is complete only when PBeam executes `Hello.main/0`, prints exactly
`Hello, world!\n` to stdout, and exits with status zero. Compilation, BEAM inspection,
and an ERTS reference run are preparation; none completes this target.

Terra Counter follows as a broader application compatibility target. The compiled ERTS
reference and both greeting variants are verified. PBeam has an initial constant-return
BEAM loader in review; it does not yet execute the Hello World entry.

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
and `Another greeting.\n`, respectively. Rust coverage remains N/A for the empty
crate. In-process tests measure fixture coverage; subprocess tests independently
verify the original compiled artifacts and process-level results.

| Kind | Instruction or operation | Required semantics | Evidence | PBeam support and limits | Task / PR / test |
| --- | --- | --- | --- | --- | --- |
| Instruction | `move literal, x0` | Put the binary literal into argument register 0 | Static entry disassembly, both modules | Only immediate atoms and integers through 2047; binary literals remain unsupported | #4; `constant.rs` |
| Instruction | `call_ext_only 1, Elixir.IO:puts/1` | External tail call with one argument; preserve caller return destination | Static final entry instruction, both modules | Not implemented | #5, #42; `hello_test.exs` |
| Loader metadata | `label`, `line`, `func_info` | Resolve entry label 11; distinguish metadata from executable operations | Static `main/0` disassembly, source lines 2–3 | Supports `label`, `func_info`, `return`, `int_code_end`; no general entry metadata | #4 |
| Terms | Binary literals, atom `ok` | Load each distinct greeting; output call returns `:ok` | Disassembly plus both in-process assertions | Atoms and non-negative integers through 2047 only; no binary literal or general term support | #4, #42 |
| External MFA | `Elixir.IO:puts/1` | Write supplied greeting plus newline to standard output | Static import; observed output and `:ok` result under ERTS | Proposed native boundary below; not implemented | #42; both process tests |
| Host operation | stdout write | Exact bytes; output errors must remain visible | Successful output observed; failure behavior still belongs to #42 | Not implemented | #42 |
| Static library dependency | `IO.puts/2`, `:io.put_chars/2` | Default-device mapping, chardata conversion, IO request handling | Installed Elixir `lib/elixir/lib/io.ex`, `puts` definition; not independently traced | Not required if proposed native boundary is accepted | #42 decision |
| Compiler-generated functions | `__info__/1`, `module_info/0,1`, helper; `:erlang.get_module_info/1,2` imports | Introspection outside the entry path | Static exports/imports; no calls from the inspected `main/0` body | Out of this fixture's execution scope | #4 loader must not mistake imports for executed dependencies |

Entry disassembly contains a literal move followed by the external tail call;
there is no stdout opcode. Static inspection is not an opcode execution trace,
and successful output does not establish the complete dynamic OTP dependency
set. The table records the bounded entry path and observed behavior, not general
OTP support.

### Proposed output boundary

For #42, resolve the ordinary external MFA `Elixir.IO:puts/1` to a minimal Rust
implementation accepting the fixture's valid UTF-8 binary, appending one newline,
writing to stdout, and returning `:ok`. Validate the supported argument form and
propagate write failures; do not recognize greeting contents or delegate to ERTS.
Keep both literal variants as regression tests.

Executing the full Elixir/OTP IO implementation would introduce device resolution,
chardata conversion, and process-based IO behavior before this checkpoint. The
narrow boundary preserves this fixture's source and observable behavior while
explicitly leaving arbitrary chardata, custom devices, and general IO unsupported.
This is a proposed decision for #42's review, not implemented compatibility.
Reuse external-MFA resolution from #5; add only the one required native mapping
in #42. The broader registration mechanism remains #12. If implementation evidence
requires independent prerequisite behavior, create sibling issues before expanding
task #42.

Each implementation PR updates affected rows with supported semantics, remaining
limits, and runnable regression tests. Mark support verified only after merge and
passing acceptance checks. GitHub links below identify the tasks; PR #43 supplies
the reference evidence and `fixtures/hello/test/hello_test.exs` contains its checks.

## Implementation checklist

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
its stdout, stderr, and exit status. The exact runner command is established by
the implementation PR; there is no working PBeam run command yet.

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
