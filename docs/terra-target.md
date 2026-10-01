# First Terra compatibility target

This contract defines the first application scenario for PBeam. Source inspection
supports the expectations below; neither an ERTS reference run on these toolchains
nor a PBeam execution has been verified. [Task #3](https://github.com/fahchen/pbeam/issues/3)
owns the reproducible reference run and compatibility inventory.

## Source and environment

- Project: [uminocelo/terra](https://github.com/uminocelo/terra).
- Revision: [`30dea5667d7e4ae5777aa3df77acb36c4aac6967`](https://github.com/uminocelo/terra/tree/30dea5667d7e4ae5777aa3df77acb36c4aac6967).
- Application: the upstream Counter, using its unmodified state, update, and view functions.
- Initial platform: macOS arm64 (`aarch64-apple-darwin` for Rust).
- Terminal: an 80-column, 24-row PTY with `TERM=xterm-256color`; record the
  macOS version, locale, PTY harness, and initial terminal modes with the reference run.
  Use the same environment for the eventual PBeam comparison.

Terra's [project metadata](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/mix.exs)
declares version `1.1.1`, Elixir `~> 1.18`, and no runtime dependencies.
Its documentation requires OTP 28 or later. `ex_doc` is development-only.

The pinned [mix.lock](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/mix.lock)
has Git blob SHA `76ff16d751c881b80e8c20cb15a347d22b5fb189`. It locks `ex_doc 0.40.4`
and its documentation dependencies (`earmark_parser`, `makeup`, `makeup_elixir`,
`makeup_erlang`, and `nimble_parsec`). Preserve that upstream lockfile when recording
the baseline; these entries do not add runtime dependencies to Counter.

## Toolchain baseline

The owner selected mise and the latest stable OTP, Elixir, and Rust. These releases
were verified on 2026-10-02 and are pinned in [mise.toml](../mise.toml):

| Tool | Pin | Release |
| --- | --- | --- |
| Erlang/OTP | `29.1.1` | [OTP-29.1.1](https://github.com/erlang/otp/releases/tag/OTP-29.1.1) |
| Elixir | `1.20.4-otp-29` | [v1.20.4](https://github.com/elixir-lang/elixir/releases/tag/v1.20.4) |
| Rust | `1.99.0` | [1.99.0](https://github.com/rust-lang/rust/releases/tag/1.99.0) |

The OTP-specific Elixir build aligns with OTP 29. The official
[Elixir compatibility table](https://elixir.hexdocs.pm/compatibility-and-deprecations.html)
lists OTP 27–29 for Elixir 1.20. This establishes a supported version combination,
not proof that this Terra revision runs on it.

From the PBeam checkout, `mise install` installs the pins and `mise exec -- <command>`
runs a command with them. When launching Terra from another checkout, retain the
PBeam toolchain environment explicitly; do not accidentally use a global or Terra-local
toolchain. The reference harness must record `erl`, `elixir`, and `rustc` versions.

Do not commit floating `latest` selectors. Resolve new stable releases in a reviewed
change and rerun the baseline before changing compatibility claims. Do not silently
downgrade tools to make the scenario pass. Rust is pinned for subsequent PBeam work;
the upstream Counter itself does not require Rust.

## Scenario contract

The upstream entry command, run in the pinned Terra checkout under the PTY, is:

```sh
mix run examples/counter.exs
```

[The entry script](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/examples/counter.exs)
loads `counter_app.exs` and calls `Terra.run(Counter)`.
[Counter's source](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/examples/counter_app.exs)
defines the following expected interaction:

| Step | Input | Observable result |
| --- | --- | --- |
| Start | None | View contains `Count: 0` and `j/k to change, q to quit`. |
| Increment | `j` | View contains `Count: 1`. |
| Increment again | `j` | View contains `Count: 2`. |
| Decrement | `k` | View contains `Count: 1`. |
| Quit | `q` | Runtime terminates normally; terminal state is restored. |

Send each key only after observing the preceding screen state. Compare the visible
screen after applying terminal control sequences, not byte-for-byte ANSI transcripts.
PIDs, reference values, timing, and redraw chunk boundaries are not assertions.

On normal quit, verify cooked input mode and signal handling are restored, the cursor
is visible, attributes are reset, and the alternate screen is left. Record the
original PTY settings and compare relevant restored settings. Capture the reference
launcher's exit status in task #3; the current source review does not establish its
exact shell exit result. A crash, unexpected final count, missing frame, or failure
to restore terminal state fails the scenario. The reference harness must use a bounded
timeout and preserve output/mode diagnostics; a timeout is a failure, not a passing
or skipped result. Exact timing thresholds belong to that harness.

No application network service or credentials are required. Toolchain/dependency
downloads during setup are separate from the Counter interaction.

## Early probe and full application acceptance

The upstream [Counter test](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/test/terra/examples_test.exs)
provides an early deterministic probe: send `jj`, check state `2` and rendered
`Count: 2`, send `k`, check state `1`, then send `q` and observe normal process exit.

The [Terra.Test helper](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/lib/terra/test.ex)
disables terminal handling and defaults to no input reader. Passing that probe proves
only the exercised application/process semantics. The real PTY interaction and
restoration are required for the first application acceptance.

The eventual PBeam executable must run the same compiled Counter after being moved
away from its build tree, without installed ERTS or loose application files. Compile
the upstream `.exs` application at build time and use a minimal compiled entrypoint;
do not require `Code.require_file`, Mix, or an Elixir compiler at runtime. That
entrypoint must preserve Counter behavior and the `Terra.run(Counter)` path.

## Compatibility requirements and limits

The stock [TTY backend](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/lib/terra/terminal/tty.ex)
uses OTP `:shell.start_interactive`, IO, and a `Port.open`/`stty` path. These are real
dependencies to inventory, not behavior already supplied by the planned Host boundary.
The reference run uses the upstream terminal backend. A later implementation may
satisfy these operations through interpreted code or Rust-native/host implementations,
but must preserve observable behavior and document that choice.

For the initial macOS target, OS-provided shell/terminal utilities are allowed; record
them in the reference inventory. A single application executable does not claim a
hermetic OS environment. Upstream [terminal tests](https://github.com/uminocelo/terra/blob/30dea5667d7e4ae5777aa3df77acb36c4aac6967/test/terra/terminal_test.exs)
are source evidence for restoration expectations, not evidence of tests run here.

This scenario does not establish full Terra or OTP compatibility, terminal resizing,
mouse/paste support, signal/crash recovery, other examples, other platforms, or
performance targets. Required process, binary, IO, and startup operations must be
discovered from the reference run and implemented in separately reviewed tasks.
