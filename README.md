# PBeam

PBeam is a planned Rust runtime for executing existing Erlang/Elixir `.beam`
artifacts without ERTS and packaging applications as single native executables.

**Status: verified Elixir references, constant-return and bounded-call slices, and the compiled Elixir and OTP IO path of Hello World through the host write.**
The Hello World run uses the boot module `pbeam_boot` of `examples/hello_world` and the pinned Elixir and OTP modules. Application build tooling is not implemented yet.
`mix pbeam.build` is a proposed command, not an available installation or build instruction.

## Design

- Interpret existing BEAM artifacts produced by the normal Erlang/Elixir toolchain.
- Start with immutable, reference-counted terms and separate binary handling.
- Run multiple scheduler threads with reductions and work stealing.
- Link Rust-native functions by module/function/arity into the executable.
- Isolate platform services behind System/Host boundaries.
- Authorize protected VM operations and host effects through the proposed Capability Host boundary. Select providers through trusted host configuration.
- Embed one application bundle in a Rust launcher.

[Hello World](docs/hello-world-target.md) is the first target: PBeam executes the
compiled Elixir entry, prints exactly `Hello, world!\n`, and exits successfully.
M2 is complete ([tracker #33](https://github.com/fahchen/pbeam/issues/33)). Task #42 runs both Hello World entries with the `pbeam` binary through the compiled Elixir and OTP IO path, and the boot module `pbeam_boot` starts the OTP IO servers. For the current work, see the M3 [tracker #34](https://github.com/fahchen/pbeam/issues/34).
[Terra Counter](docs/terra-target.md) follows as a broader macOS arm64 PTY scenario. Full OTP
compatibility, hot code upgrades, JIT/AOT, and traditional NIF ABI compatibility
are outside the initial scope. Each executable targets a particular platform;
system libraries may still be required.

See the [runtime specification](docs/portable-beam-spec.md) for architecture,
acceptance criteria, and unresolved decisions.

The [capability security design](docs/capability-security-design.md) records the proposed authority, provider, and native trust contracts. The separate [migration and implementation guide](docs/capability-security-migration.md) starts with the delivered M2 terminal IO path after M2 acceptance. These documents do not claim an implemented sandbox or change the current M2 acceptance gate.

Development toolchains are pinned in [mise.toml](mise.toml). Use `mise install`
to install them; see the [target contract](docs/terra-target.md#toolchain-baseline)
for versions and verification limits. There is no PBeam build command yet.

The Rust build compiles the PCRE2 10.47 sources of OTP 29.1.1 in `vendor/pcre2` with the host C compiler, under the licence in [vendor/pcre2/LICENCE](vendor/pcre2/LICENCE).

## Checks

After installing the mise tools, install the Rust checking components and run:

```sh
mise run setup
mise run check
```

The shared local/CI command checks Markdown, TOML formatting, GitHub Actions, shell scripts,
Rust formatting, Clippy with warnings denied, tests, and 100% line coverage.
Elixir fixtures use formatting, warnings-as-errors, and Mix tests with 100% application
line coverage. The constant-return BEAM fixture is checked in at
`fixtures/beam/constant.beam`. Run `mise run beam-inventory` to inspect its disassembly,
`mise run reference` for the ERTS checks, and `mise run inventory` for the compiled Hello
World entry instructions and imports.
See the [evidence table](docs/hello-world-target.md).

## Delivery

Work is managed in [GitHub Issues](https://github.com/fahchen/pbeam/issues),
[milestones](https://github.com/fahchen/pbeam/milestones), and the
[project board](https://github.com/users/fahchen/projects/2):

**Milestone tracking issue → epic issue → task issue.**

Each milestone has a dedicated tracking issue. Epics and tasks are assigned to
the GitHub milestone; native sub-issue links express the hierarchy and native
blocked-by links express dependencies. Milestone trackers and epics list their
issues as linked bullets. For the current work, see
[the M3 tracking issue #34](https://github.com/fahchen/pbeam/issues/34).

Milestone titles use `[m1] Title`. Epic and task titles use `[m1-e1] Title`;
GitHub issue numbers identify individual tasks.

Implementation proceeds through small PRs linked to task issues with
`Closes #<issue>`. Each PR records implementation decisions, alternatives,
rationale, tradeoffs, and verification. Prerequisite PRs must be reviewed and
merged before dependent work begins. Issues and milestone checkpoints remain
open until their acceptance evidence is reviewed.

There are no file-based task plans or duplicate progress checklists. Status
tracking lives on the [project board](https://github.com/users/fahchen/projects/2).
Follow [AGENTS.md](AGENTS.md) when contributing and use the
[PR template](.github/pull_request_template.md) for review.
