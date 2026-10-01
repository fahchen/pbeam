# First target: PBeam prints Hello World

The first target is complete only when PBeam executes `Hello.main/0`, prints exactly
`Hello, world!\n` to stdout, and exits with status zero. Compilation, BEAM inspection,
and an ERTS reference run are preparation; none completes this target.

Terra Counter follows as a broader application compatibility target. The PBeam
crate is still empty. An exploratory ERTS run printed the greeting successfully,
but PBeam execution has not been implemented or verified.

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

Task #41 records the compiled program's requirements here as a table. Include
BEAM instructions, external MFA calls, BIFs, term representations, and host IO
operations separately; an external call is not itself an instruction. Record the
artifact, pinned toolchain, rebuild command, and inspection/run command alongside
the table so another reviewer can reproduce the evidence.

| Kind | Instruction or operation | Required semantics | Evidence | PBeam support and limits | Task / PR / test |
| --- | --- | --- | --- | --- | --- |
| Inventory pending | Not yet inspected | To be established by #41 | Exploratory ERTS output only; no compiled inventory yet | No runtime implementation | [#41](https://github.com/fahchen/pbeam/issues/41) |

Replace the pending row with evidence-backed entries during #41. Distinguish
statically present operations from operations observed during execution and
dependencies whose necessity is still unverified. For instructions, identify
operand forms and the relevant function/disassembly location. Do not infer a
complete runtime dependency set from the entry module's disassembly alone.

Each implementation PR updates the affected rows with supported semantics,
remaining limits, and a runnable regression test. Mark support verified only
after the implementing PR is merged and its acceptance checks pass. A passing
fixture does not imply support for every operand form or general OTP behavior.

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
must also replace the current fail-closed Elixir guard with its real checks. Passing
Hello World establishes this program's behavior, not general Elixir/OTP support.

After this checkpoint, continue the planned runtime, native, and packaging slices.
Then capture and implement the [Terra Counter contract](terra-target.md), retaining
the Hello World test as a regression check.
