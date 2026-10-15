# Plug Hello

This example is a Mix project with the dependencies `plug` and `bandit`. The versions are pinned in `mix.lock`. The plug `PlugHello` answers `GET /` with the status 200, the content type `text/plain`, and the body `Hello world!`. It answers each other request with the status 404. The application `PlugHello.Application` starts Bandit with `PlugHello` on the port of the environment variable `PORT`, default 4000. The test run uses the port 0, so that it does not need the port 4000.

## Third-party code

The build fetches the dependencies of `mix.lock` from Hex and compiles them. The Elixir compiler runs the macros of the dependencies when it compiles them, and a run then runs their code. Read the sources of the pinned versions before the first build.

## Gates

`mise run check` runs the gates of this example:

- `mise run lint` runs `mix format --check-formatted` in this directory.
- `scripts/test.sh` installs Hex and rebar3 when they are missing, and runs `mix deps.get --check-locked`. It then runs `mix test --cover --warnings-as-errors`, which compiles with warnings as errors, runs the tests, and requires 100% line coverage (`test_coverage` in `mix.exs`). One test sends a request over TCP to the Bandit listener that the application starts.
- `scripts/test.sh` then runs `MIX_ENV=prod mix release --overwrite`. The release of the pinned Elixir and OTP goes to `target/examples/plug_hello/`, which is not committed. A run needs only that directory.

## Run on the pinned ERTS

From the repository root, after `mise run check` or the build steps of `scripts/test.sh`, these commands start the release, send a request, and stop the release:

```sh
target/examples/plug_hello/bin/plug_hello daemon
curl -s --retry 20 --retry-connrefused --retry-delay 1 http://127.0.0.1:4000/
target/examples/plug_hello/bin/plug_hello stop
```

The `curl` command prints `Hello world!`. Use `PORT=<port>` before `daemon` to use another port.

## Run on the pbeam binary

The `pbeam` binary does not read a boot script, so it cannot run `bin/plug_hello`. The run of task #652 used `pbeam_boot` of `examples/hello_world` with `--root target/examples/plug_hello`, one `--pa` for each `lib/*/ebin` directory and for `releases/0.1.0/consolidated`, and an entry that calls `application:ensure_all_started(plug_hello)`. The run does not serve the page yet. Issue #652 lists the stops of that run in order.
