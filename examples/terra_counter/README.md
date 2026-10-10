# Terra Counter

This example runs the Counter application of [uminocelo/terra](https://github.com/uminocelo/terra) at the pinned commit `30dea5667d7e4ae5777aa3df77acb36c4aac6967` of the [Terra target](../../docs/terra-target.md). `TerraCounter.main/0` of `terra_counter.ex` calls `Terra.run(Counter)`, as the upstream script `examples/counter.exs` does, and prints the result with `IO.inspect/1` after Terra gives back the terminal. Counter shows `Count: 0`. The key `j` adds 1, the key `k` subtracts 1, and the key `q` quits. `Terra.run/1` returns `{:ok, final_state}` when the application quits (`lib/terra.ex` of Terra), so the keys `j j k q` are expected to print `{:ok, 1}`. A run of this example has not recorded this result yet; task [#3](https://github.com/fahchen/pbeam/issues/3) records the reference run.

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

The `pbeam` binary loads the Terra modules, `Counter`, and `TerraCounter`, and each pinned OTP and Elixir module that they and the boot call, directly or through other modules, with the consolidated protocols in place of the unconsolidated protocols of Elixir and the implementations of each consolidated protocol that they call. For example, Terra calls `Code.ensure_loaded/1` (`lib/terra/runtime.ex:121` and `:161`), and the Elixir module `Code` calls `code:ensure_loaded/1`, so the run gets `code`. The run does not get `code_server`, because PBeam has no code server. Thus `code:ensure_loaded/1` of a module that is not loaded stops the run with `unresolved external MFA code_server:get_mode/0`. The task writes each called module that the run does not get, with the reason, into `target/examples/terra_counter/pbeam.unloaded`.
