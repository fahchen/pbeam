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
M2 is complete ([tracker #33](https://github.com/fahchen/pbeam/issues/33)). Task #42 runs both Hello World entries with the `pbeam` binary through the compiled Elixir and OTP IO path, and the boot module `pbeam_boot` starts the OTP IO servers. For the milestone order and the current work, see [Delivery](#delivery).
[Terra Counter](docs/terra-target.md) follows as a broader macOS arm64 PTY scenario. Full OTP
compatibility, hot code upgrades, JIT/AOT, and traditional NIF ABI compatibility
are outside the initial scope. Each executable targets a particular platform;
system libraries may still be required.

See the [runtime specification](docs/portable-beam-spec.md) for architecture,
acceptance criteria, and unresolved decisions.

The [capability security design](docs/capability-security-design.md) records the proposed authority, provider, and native trust contracts. The separate [migration and implementation guide](docs/capability-security-migration.md) starts with the delivered M2 terminal IO path after M2 acceptance. These documents do not claim an implemented sandbox. Milestone M6 ([tracker #304](https://github.com/fahchen/pbeam/issues/304)) implements them.

Development toolchains are pinned in [mise.toml](mise.toml). Use `mise install`
to install them; see the [target contract](docs/terra-target.md#toolchain-baseline)
for versions and verification limits. There is no PBeam build command yet.

The Rust build compiles the PCRE2 10.47 sources of OTP 29.1.1 in `vendor/pcre2` with the host C compiler, under the licence in [vendor/pcre2/LICENCE](vendor/pcre2/LICENCE).

## Examples

Each directory in [examples](examples) holds one example program. From the repository root, this command builds the Hello World example and runs it with the `pbeam` binary. It prints `Hello, world!` and exits with status 0:

```sh
mise run example hello_world
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`. The `+S 1` flag gives ERTS one scheduler, as the `pbeam` binary has one scheduler thread. Run `mise run example --help` to print the arguments. The task compiles the Elixir sources `examples/<name>/*.ex` and the boot module `examples/hello_world/pbeam_boot.erl` into `target/examples/<name>/`. It consolidates the protocols of Elixir into `target/examples/<name>/consolidated/`, as `mix run` does. It then runs `main/0` of the module whose name is the camel-case name of the directory, for example `HelloWorld` for `hello_world`. The `pbeam` binary loads the example modules, the pinned Elixir modules that the example modules call, directly, through other Elixir modules, or through a consolidated protocol, the 41 pinned OTP modules of the boot, the pinned OTP modules that the example modules call directly, and `pbeam_boot`. The variable `PBEAM_BIN` selects the `pbeam` binary. Without it, the task uses `cargo run`. The [Hello World example](examples/hello_world/README.md) shows the commands that the task runs. The [Terra Counter example](examples/terra_counter/README.md) also compiles the Terra sources of the pinned commit, which the task downloads. The first build runs this third-party code.

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

In CI, a pull request that changes only Markdown files runs `mise run lint` instead of `mise run check`. A Markdown file that a test or build step reads counts as code: `scripts/ci_task.sh` lists these files. A push to `main` always runs `mise run check`.

## Delivery

Work is managed in [GitHub Issues](https://github.com/fahchen/pbeam/issues),
[milestones](https://github.com/fahchen/pbeam/milestones), and the
[project board](https://github.com/users/fahchen/projects/2):

**Milestone tracking issue → epic issue → task issue.**

Each milestone has a dedicated tracking issue. Epics and tasks are assigned to the GitHub milestone; native sub-issue links express the hierarchy and native blocked-by links express dependencies. Milestone trackers and epics list their issues as linked bullets.

The owner decision of 2026-10-09 sets the order of the open milestones. M5 and M6 run in parallel.

1. M4 Tooling and examples ([tracker #303](https://github.com/fahchen/pbeam/issues/303))
2. M5 Interactive host I/O ([tracker #38](https://github.com/fahchen/pbeam/issues/38))
3. M6 Capability security ([tracker #304](https://github.com/fahchen/pbeam/issues/304))
4. M7 Linked native calls ([tracker #36](https://github.com/fahchen/pbeam/issues/36))
5. M8 Embedded application ([tracker #37](https://github.com/fahchen/pbeam/issues/37))
6. M9 Pinned Terra scenario ([tracker #39](https://github.com/fahchen/pbeam/issues/39))
7. M10 Multiple schedulers ([tracker #35](https://github.com/fahchen/pbeam/issues/35))

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
