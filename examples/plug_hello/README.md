# Plug Hello

This example is a Mix project with the dependencies `plug` and `bandit`. The versions are pinned in `mix.lock`. The plug `PlugHello` answers `GET /` with the status 200, the content type `text/plain`, and the body `Hello world from PBeam!`, on the pinned ERTS and on the `pbeam` binary. It answers each other request with the status 404. The application `PlugHello.Application` starts Bandit with `PlugHello` on the port of the environment variable `PORT`, default 4000. The test run uses the port 0, so that it does not need the port 4000.

## Build one executable

From this directory, after `mix release`, this command writes `plug_hello.bundle` under `target/examples/` of the repository:

```sh
mix pbeam.build
```

The task comes from the `pbeam` path dependency of `mix.exs`. It builds the `pbeam` runtime and the `pbeam_bundle` tool with Cargo in the repository, writes the bundle with `mix pbeam.bundle`, and appends the bundle to the runtime. The executable runs in a directory that holds no other file, and the run needs no installed OTP. `--output <path>` names another output path, and `--runtime <path>` and `--tool <path>` use built binaries instead of the Cargo build.

From the repository root, this task writes the same executable for each example:

```sh
mise run example-executable plug_hello
```

It builds the example, the runtime, and the bundle tool, and writes `target/examples/plug_hello.bundle`. `--no-build` uses the build that is already there, and `--output <path>` names the output path. A release example uses the entry `elixir:start_cli`, which the launcher gives to the boot of the release as `-s elixir start_cli`, as `bin/plug_hello start` does. A fixture example, such as `hello_world`, uses `main/0` of its module.

## Third-party code

The build fetches the dependencies of `mix.lock` from Hex and compiles them. The Elixir compiler runs the macros of the dependencies when it compiles them, and a run then runs their code. Read the sources of the pinned versions before the first build.

## Gates

`mise run check` runs the gates of this example:

- `mise run lint` runs `mix format --check-formatted` in this directory.
- `scripts/test.sh` installs Hex and rebar3 when they are missing, and runs `mix deps.get --check-locked`. It then runs `MIX_ENV=test mix compile --warnings-as-errors`, which compiles the application code with warnings as errors, because `mix test --warnings-as-errors` does not pass the flag to that compile. Then `mix test --cover --warnings-as-errors` runs the tests with warnings as errors in the test files, and requires 100% line coverage (`test_coverage` in `mix.exs`). One test sends a request over TCP to the Bandit listener that the application starts.
- `scripts/test.sh` then runs `MIX_ENV=prod mix release --overwrite`. The release of the pinned Elixir and OTP goes to `target/examples/plug_hello/`, which is not committed. A run needs only that directory.

## Run on the pinned ERTS

From the repository root, after `mise run check` or the build steps of `scripts/test.sh`, these commands start the release, send a request, and stop the release:

```sh
target/examples/plug_hello/bin/plug_hello daemon
curl -s --retry 20 --retry-connrefused --retry-delay 1 http://127.0.0.1:4000/
target/examples/plug_hello/bin/plug_hello stop
```

The `curl` command prints `Hello world from PBeam!`. Use `PORT=<port>` before `daemon` to use another port.

## Run with the example runner

The example runner boots the release with its boot script `releases/0.1.0/start.boot`, as `bin/plug_hello start` does with `RELEASE_DISTRIBUTION=none`. Both runs get the same `erl` arguments:

```text
-noshell -s elixir start_cli -mode embedded -config <release>/releases/0.1.0/sys -boot <release>/releases/0.1.0/start -boot_var RELEASE_LIB <release>/lib -extra --no-halt
```

The ERTS run gives them to the `erl` of the release, `erts-17.1/bin/erl`. The pbeam run gives them to `pbeam erl --root <release> --bindir <release>/erts-17.1/bin --preloaded <ebin directory of the pinned erts>`. The arguments omit `-setcookie`, which only the distribution reads, and `-args_file` of `vm.args`, which `pbeam erl` rejects; the `vm.args` of `mix release` holds only comments. Both runs get `RELEASE_ROOT` and `RELEASE_SYS_CONFIG`, which the release boot reads. `pbeam erl` rejects a set, non-empty `ERL_AFLAGS`, `ERL_FLAGS`, or `ERL_ZFLAGS`: unset them first. The "Release boot" row of [ERTS Differences](../../docs/erts-differences.md) gives the differences.

From the repository root, these commands boot the release on the pinned ERTS, send a request, and stop the run:

```sh
mise run example plug_hello --erts &
curl -s --retry 60 --retry-connrefused --retry-delay 1 http://127.0.0.1:4000/
kill %1
```

The `curl` command prints `Hello world from PBeam!`. Use `PORT=<port>` before `mise run` to use another port. The task builds the release with `mix deps.get --check-locked` and `MIX_ENV=prod mix release --overwrite`, and writes the output of `mix` to stderr. Hex and rebar3 must be installed, as `scripts/test.sh` installs them.

This command boots the same release on the `pbeam` binary, sends a request, and stops the run:

```sh
mise run example plug_hello &
curl -s --retry 60 --retry-connrefused --retry-delay 1 http://127.0.0.1:4000/
kill %1
```

The `curl` command prints `Hello world from PBeam!`. The run does not end by itself, so `kill` stops it.

The release runner serves the same page:

```sh
mise run example-release plug_hello &
curl -s --retry 300 --retry-connrefused --retry-delay 1 -D - http://127.0.0.1:4000/
kill %1
```

The `curl` command prints the status line `HTTP/1.1 200 OK`, the headers of the response, and the body `Hello world from PBeam!`. The feature test `serves_hello_world_from_the_plug_example_as_pinned_erts` of `tests/feature/serve.rs` compares the status line, the headers without `date`, and the body of the ERTS run, the pbeam run, and the one-file executable. It also boots the release with an `-eval` probe on both runtimes and compares the NIF record: `erlang:module_loaded/1` of each module of a PBeam NIF library and of `pubkey_os_cacerts`.

A request whose `Accept-Encoding` accepts `zstd` gets a body that Bandit compresses with `zstd:compress/2` and a context. PBeam binds the `zstd` context NIFs with libzstd 1.5.7, the version of the pinned ERTS, so the compressed bytes are the same. The feature test sends such a request first and compares its response with the ERTS response.
