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

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`. The `+S 1` flag gives ERTS one scheduler, as the `pbeam` binary has one scheduler thread. Run `mise run example --help` to print the arguments. The task compiles the Elixir sources `examples/<name>/*.ex` and the boot module `examples/hello_world/pbeam_boot.erl` into `target/examples/<name>/`. It consolidates the protocols of Elixir into `target/examples/<name>/consolidated/`, as `mix run` does. It then runs `main/0` of the module whose name is the camel-case name of the directory, for example `HelloWorld` for `hello_world`. The build also writes the code path of the ERTS run into `target/examples/<name>/erts.paths`, the start set of the `pbeam` run into `target/examples/<name>/pbeam.start`, and the whole list of the build into `target/examples/<name>/pbeam.beams`. The command `bash scripts/example.sh <name> [--erts] --no-build` does not build: it runs the last build with these lists. The tests use it, so that each test builds its example once and its ERTS run and `pbeam` run execute the same compiled modules. The `pbeam` binary loads the start set: the example modules, `pbeam_boot`, the 44 pinned OTP modules of the boot, and the preloaded modules of ERTS that PBeam loads from a BEAM file. Each other module loads on demand through the pinned `error_handler` and `code_server`, as in an interactive-mode ERTS run. The build writes each called module that the PBeam loader does not load, with the reason, into `target/examples/<name>/pbeam.unloaded`. For example, the PBeam loader does not load `crypto` (`unsupported BEAM opcode 149 (on_load)`). The task gives the `pbeam` binary `--root` with the root directory of the pinned OTP, so `pbeam_boot` starts `code_server`, and `--pa` with each directory of the code path. Add `--policy <module>` to run the `pbeam` binary with a policy module of the example, as the [Restricted Guest example](examples/restricted_guest/README.md) does. The ERTS run has no policy, so the task rejects `--policy` with `--erts`. The variable `PBEAM_BIN` selects the `pbeam` binary. Without it, the task uses `cargo run`. Run `mise run example-release <name>` to build the release `pbeam` binary with `cargo build --locked --release --bin pbeam` and run the example on it. That task takes `--policy <module>` and no `--erts`. The [Hello World example](examples/hello_world/README.md) shows the commands that the task runs. The [Terra Counter example](examples/terra_counter/README.md) also compiles the Terra sources of the pinned commit, which the task downloads. The first build runs this third-party code. A directory with a `mix.exs` is a release example, such as the [Plug Hello example](examples/plug_hello/README.md). For it, the task builds the Mix release into `target/examples/<name>/`, and runs `main/0` of the Erlang module `<name>_run` of `examples/<name>/<name>_run.erl` with the code path of the release. That example README gives the commands.

The `pbeam` binary also takes `--pa DIR`, with the name of the `erl` flag. Repeat it to give more directories. With `--pa`, the binary gives the root function one more argument after the atoms: the boot flag list `[{pa, [Dir]}]`, one tuple for each `--pa` in the command line order, with each directory as a binary. `pbeam_boot:start/3` takes this list and starts the pinned `erl_prim_loader` with the code path of the directories and `"."`, with the last `--pa` directory first, as `erl -pa DIR` does. The path command of `start_clean.boot` then adds the `ebin` directories of `kernel` and `stdlib` of the `--root` directory after them, and `erl_prim_loader:get_file/1` finds a file in these directories (see "Boot" in [ERTS Differences](docs/erts-differences.md)). To try it, add `--pa DIR` before `--` in the `pbeam` command of the [Hello World example](examples/hello_world/README.md). The test `starts_erl_prim_loader_with_the_code_path_of_pa_as_pinned_erts` in `tests/feature/prim_loader.rs` compares `erl_prim_loader:get_path/0`, `get_file/1`, and `read_file/1` with `erl -boot start_clean -pa DIR`.

The `pbeam` binary also takes `--root DIR` and `--mode interactive|embedded`, with the names of the `erl` flags. `--root` gives the root directory of the OTP installation, as `erl` gives `-root`, and has no default. With it, the boot flag list starts with `{root, [Root]}`. `--mode` needs `--root`, and adds `{mode, [Mode]}` at the end of the list. Without `--mode`, the mode is `interactive`, as for `erl`. `pbeam_boot` needs `--root`, as `init` needs `-root`: without it, the boot exits with `no_or_multiple_root_variables`. `pbeam_boot` sets the code path of the pinned `start_clean.boot`, the `--pa` directories and the `ebin` directories of `kernel` and `stdlib`, and starts the `kernel` application, whose `kernel_sup` starts the pinned `code_server` with `code:start_link/0` as its first child. Then `code:ensure_loaded/1` loads a module from the code path in interactive mode, and returns `{error, embedded}` in embedded mode, and `code:which/1` and `code:get_path/0` give the ERTS results. The [Hello World example](examples/hello_world/README.md) shows the `pbeam` command with `--root`. In embedded mode, the start BEAM files must hold each module of the kernel start, because `code_server` loads no module on demand. The tests in `tests/feature/code_server.rs` compare the results with `erl -boot start_clean`, in both modes. See "Boot" and "Code loading" in [ERTS Differences](docs/erts-differences.md).

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

The owner decisions of 2026-10-15 set the order of the open milestones. M11 follows M5. The owner decision of 2026-10-19 postponed M6 Capability security again after M9 closed, so M10 Multiple schedulers is next. M6 resumes only after a new owner decision. The landed capability checks of epic [#305](https://github.com/fahchen/pbeam/issues/305) stay, and M7 to M9 keep the accepted security contracts. The milestone numbers do not change.

1. M5 Interactive host I/O ([tracker #38](https://github.com/fahchen/pbeam/issues/38))
2. M11 Serve HTTP with Plug and Bandit ([tracker #650](https://github.com/fahchen/pbeam/issues/650))
3. M7 Linked native calls ([tracker #36](https://github.com/fahchen/pbeam/issues/36))
4. M8 Embedded application ([tracker #37](https://github.com/fahchen/pbeam/issues/37))
5. M9 Pinned Terra scenario ([tracker #39](https://github.com/fahchen/pbeam/issues/39))
6. M10 Multiple schedulers ([tracker #35](https://github.com/fahchen/pbeam/issues/35))
7. M6 Capability security ([tracker #304](https://github.com/fahchen/pbeam/issues/304)) — postponed until a new owner decision

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
