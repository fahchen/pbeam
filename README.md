# PBeam

PBeam is a planned Rust runtime for executing existing Erlang/Elixir `.beam`
artifacts without ERTS and packaging applications as single native executables.

**Status: verified Elixir reference fixtures and an empty Rust library scaffold.** The interpreter, runtime,
and application build tooling are not implemented yet. `mix pbeam.build` is a proposed command,
not an available installation or build instruction.

## Design

- Interpret existing BEAM artifacts produced by the normal Erlang/Elixir toolchain.
- Start with immutable, reference-counted terms and separate binary handling.
- Run multiple scheduler threads with reductions and work stealing.
- Link Rust-native functions by module/function/arity into the executable.
- Isolate platform services behind System/Host boundaries.
- Embed one application bundle in a Rust launcher.

[Hello World](docs/hello-world-target.md) is the first target: PBeam must execute the
Elixir program, print exactly `Hello, world!\n`, and exit successfully. Compilation
and an ERTS smoke run are preparation, not completion. PBeam execution is pending.
[Terra Counter](docs/terra-target.md) follows as a broader macOS arm64 PTY scenario. Full OTP
compatibility, hot code upgrades, JIT/AOT, and traditional NIF ABI compatibility
are outside the initial scope. Each executable targets a particular platform;
system libraries may still be required.

See the [runtime specification](docs/portable-beam-spec.md) for architecture,
acceptance criteria, and unresolved decisions.

Development toolchains are pinned in [mise.toml](mise.toml). Use `mise install`
to install them; see the [target contract](docs/terra-target.md#toolchain-baseline)
for versions and verification limits. There is no PBeam build command yet.

## Checks

After installing the mise tools, install the Rust checking components and run:

```sh
mise exec -- rustup component add rustfmt clippy llvm-tools-preview --toolchain 1.99.0
mise run check
```

The shared local/CI command checks Markdown, TOML formatting, GitHub Actions, shell scripts,
Rust formatting, Clippy with warnings denied, tests, and 100% line coverage.
The initial `pbeam-runtime` crate contains only a module comment: it has no
implementation, tests, or measurable coverage. Coverage is N/A until code exists.
Elixir fixtures use formatting, warnings-as-errors, and Mix tests with 100% application
line coverage. Run `mise run reference` for the ERTS checks and `mise run inventory`
for the compiled entry instructions/imports. See the [evidence table](docs/hello-world-target.md).

## Delivery

Work is managed in [GitHub Issues](https://github.com/fahchen/pbeam/issues)
and [milestones](https://github.com/fahchen/pbeam/milestones):

**Milestone tracking issue → epic issue → task issue.**

Each milestone has a dedicated tracking issue. Epics and tasks are assigned to
the GitHub milestone; native sub-issue links express the hierarchy and native
blocked-by links express dependencies. Milestone trackers and epics list their
issues as linked bullets. Start with
[the first milestone](https://github.com/fahchen/pbeam/issues/32).

Milestone titles use `[m1] Title`. Epic and task titles use `[m1-e1] Title`;
GitHub issue numbers identify individual tasks.

Implementation proceeds through small PRs linked to task issues with
`Closes #<issue>`. Each PR records implementation decisions, alternatives,
rationale, tradeoffs, and verification. Prerequisite PRs must be reviewed and
merged before dependent work begins. Issues and milestone checkpoints remain
open until their acceptance evidence is reviewed.

There are no file-based task plans or duplicate progress checklists. Follow
[AGENTS.md](AGENTS.md) when contributing and use the
[PR template](.github/pull_request_template.md) for review.
