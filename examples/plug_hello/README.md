# Plug Hello

This example is a Mix project with the dependencies `plug` and `bandit`. The versions are pinned in `mix.lock`. The plug `PlugHello` answers `GET /` with the status 200, the content type `text/plain`, and the body `Hello world!`. It answers each other request with the status 404. The application `PlugHello.Application` starts Bandit with `PlugHello` on the port of the environment variable `PORT`, default 4000. The test run uses the port 0, so that it does not need the port 4000.

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

The `curl` command prints `Hello world!`. Use `PORT=<port>` before `daemon` to use another port.

## Run with the example runner

The `pbeam` binary does not read a boot script, so it cannot run `bin/plug_hello`. The example runner runs the release code without the boot script, on the pinned ERTS and on the `pbeam` binary, with the same code path and the same application environment. From the repository root, these commands start the release code on the pinned ERTS, send a request, and stop the run:

```sh
mise run example plug_hello --erts &
curl -s --retry 60 --retry-connrefused --retry-delay 1 http://127.0.0.1:4000/
kill %1
```

The `curl` command prints `Hello world!`. Use `PORT=<port>` before `mise run` to use another port.

The task builds the release with `mix deps.get --check-locked` and `MIX_ENV=prod mix release --overwrite`, and writes the output of `mix` to stderr. Hex and rebar3 must be installed, as `scripts/test.sh` installs them. The task then compiles `plug_hello_run.erl` and `examples/hello_world/pbeam_boot.erl` into `target/examples/plug_hello/pbeam/`. The code path is that directory, each `lib/*/ebin` directory of the release, and `releases/0.1.0/consolidated`. The task sets `RELEASE_ROOT` and `RELEASE_SYS_CONFIG` as `bin/plug_hello` sets them. The entry `plug_hello_run:main/0` does the steps of the boot script that set the application environment and start the applications:

1. It reads `releases/0.1.0/sys.config` as `application_controller` reads a `-config` file, and sets each value with `application:set_env/2` and `{persistent, true}`.
2. It calls `'Elixir.Config.Provider':boot/0`, as the boot script does after the start of `kernel` and `stdlib`. This evaluates `releases/0.1.0/runtime.exs`, which sets the key `port` of `plug_hello` from `PORT`, default 4000.
3. It calls `application:ensure_all_started(plug_hello, permanent)` and waits.

The run is in interactive mode. The release runs in embedded mode. The "Boot" row of [ERTS Differences](../../docs/erts-differences.md) gives the differences.

This command runs the same entry on the `pbeam` binary, sends a request, and stops the run:

```sh
mise run example plug_hello &
curl -s --retry 60 --retry-connrefused --retry-delay 1 http://127.0.0.1:4000/
kill %1
```

The `curl` command prints `Hello world!`. The run does not end by itself, so `kill` stops it.

The release runner serves the same page:

```sh
mise run example-release plug_hello &
curl -s -D - http://127.0.0.1:4000/
kill %1
```

The `curl` command prints the status line `HTTP/1.1 200 OK`, the headers of the response, and the body `Hello world!`. The `scripts/example.sh` runner starts the release code on the pinned ERTS and on the `pbeam` binary. The feature test `serves_hello_world_from_the_plug_example_as_pinned_erts` of `tests/feature/serve.rs` compares the status line, the content type, and the body of both runs.
