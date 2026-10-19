# Terra Counter

This example runs the Counter application of [uminocelo/terra](https://github.com/uminocelo/terra) at the pinned commit `30dea5667d7e4ae5777aa3df77acb36c4aac6967` of the [Terra target](../../docs/terra-target.md). `TerraCounter.main/0` of `terra_counter.ex` calls `Terra.run(Counter)`, as the upstream script `examples/counter.exs` does, and prints the result with `IO.inspect/1` after Terra gives back the terminal. Counter shows `Count: 0`. The key `j` adds 1, the key `k` subtracts 1, and the key `q` quits. `Terra.run/1` returns `{:ok, final_state}` when the application quits (`lib/terra.ex` of Terra), so the keys `j j k q` print `{:ok, 1}`. The [Terra reference](../../docs/terra-reference.md) of task [#3](https://github.com/fahchen/pbeam/issues/3) records this result on the pinned ERTS with `mise run terra-reference`.

This directory holds only the entry module and this README. The repository has no Terra source and no compiled Terra file.

## Third-party code

The first build runs third-party code. The Elixir compiler runs the macros of the Terra sources (for example `use Terra`) when it compiles them, and each run then runs the Terra code. Read the Terra sources at the pinned commit before the first build. Run the commands yourself, or allow them to an agent only after this review.

## Run

From the repository root, in a terminal, this command builds the example and runs it with the `pbeam` binary:

```sh
mise run example terra_counter
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`:

```sh
mise run example terra_counter --erts
```

Terra reads the keys from the terminal in raw mode, so run the commands in a terminal. The Terra target uses a terminal of 80 columns and 24 rows, `TERM=xterm-256color`, and a UTF-8 locale, for example `LANG=en_US.UTF-8`.

## Build

The task does the steps of the other examples (see the [Hello World example](../hello_world/README.md)). Before it compiles `terra_counter.ex`, it does these steps:

1. `scripts/terra.sh` finds the Terra sources. Without the variable `PBEAM_TERRA_SRC`, it downloads the pinned commit from GitHub into `target/terra/<commit>/` with `git fetch --depth 1`, once. With `PBEAM_TERRA_SRC`, it uses that local Terra git repository and does not change it. In both cases, it stops with status 2 when `HEAD` is not the pinned commit, or when `lib/` or `examples/counter_app.exs` has changed, untracked, or ignored files.
2. `elixirc` compiles the Terra sources `lib/**/*.ex` and the Counter application `examples/counter_app.exs` into `target/examples/terra_counter/`. Mix is not used, because the Terra `mix.exs` needs Hex for the development dependency `ex_doc`.

After it compiles `terra_counter.ex`, the task consolidates the protocols of Elixir with the implementations of Elixir, Terra, `Counter`, and `TerraCounter` into `target/examples/terra_counter/consolidated/`, as `mix run` does in the Terra project. Thus `String.Chars` (`"Count: #{count}"`) and `Inspect` (`IO.inspect/1`) find their implementations with a pattern match and do not call `code:ensure_loaded/1`. The run on ERTS puts the consolidated protocols first in the code path.

The `pbeam` binary loads a start set: the Terra modules, `Counter`, `TerraCounter`, `pbeam_boot`, the boot modules, and the consolidated protocols in place of the unconsolidated protocols of Elixir. Each other pinned OTP and Elixir module loads on demand through the pinned `error_handler` and `code_server`, as in an interactive-mode ERTS run. For example, Terra calls `Code.ensure_loaded/1` (`lib/terra/runtime.ex:121` and `:161`), and the Elixir module `Code` calls `code:ensure_loaded/1`. The run gives `--root`, so `pbeam_boot` starts `code_server`, and `--pa` with the code path of the build. Thus `main/0` waits for the 11 module loads of its first `IO.inspect/1` call, and the terminal owner writes the restore sequence before the result, as an interactive-mode ERTS run does (#428). The task writes each called module that the PBeam loader does not load, with the reason, into `target/examples/terra_counter/pbeam.unloaded`.

## Build one executable

From the repository root, this command builds the example and writes one executable:

```sh
mise run example-executable terra_counter
```

The task writes `target/examples/terra_counter.bundle`, or the path of `--output`. The file holds the `pbeam` runtime, the compiled Terra sources, `Counter`, `TerraCounter`, and the OTP and Elixir modules of the run. Move the file into an empty directory and run it there in a terminal. The run needs no Erlang or Elixir installation, no release directory, and no other file of the build. It needs the host `sh` and `stty` utilities on `PATH`, `TERM=xterm-256color`, and a UTF-8 locale, as the Terra target requires. The keys `j j k q` print `{:ok, 1}` after Terra gives back the terminal, and the run exits with status 0.

The check of this slice is the Terra Counter test in `tests/feature/example.rs`, `restores_the_terminal_before_the_result_of_terra_counter_as_interactive_erts`. It builds the executable from the same build, runs it in a directory that holds only the executable, with an empty `HOME` and a `PATH` that holds no `erl` or `elixir`, and compares the result and the exit status with the pinned ERTS on the same build. The check ran on macOS arm64 on 2026-10-19.

Scope of the claim:

- Terra `30dea5667d7e4ae5777aa3df77acb36c4aac6967`, OTP 29.1.1, ERTS 17.1, and Elixir 1.20.4 as `mise.toml` pins them.
- macOS arm64. The bundle carries BEAM files and the host build of the `pbeam` binary, so the executable runs only on the platform and architecture of that build. The Terra scenario check does not run on Linux, and this README makes no Linux claim.
- The keys `j j k q` of the Terra reference. The check does not repeat the resize and flush steps of the reference harness, and it does not claim them for the executable.
