#!/usr/bin/env bash
# Run the tests of the Mix projects with their gates, and build the Elixir outputs that the Rust tests read.
set -euo pipefail

# The root Mix project holds the PBeam.Native library. It declares the native
# functions of an Elixir module that PBeam runs. The test requires 100%
# coverage of the library.
mix test --cover --warnings-as-errors

(cd fixtures/hello && mix test --cover --warnings-as-errors)
# The Plug and Bandit example fetches the dependencies of its mix.lock from Hex, so the check installs Hex and rebar3 (for telemetry) when they are missing.
# `mix test --warnings-as-errors` does not pass the flag to the compile of the application code, so the check compiles the application first with warnings as errors. The test requires 100% coverage. The release build writes the compiled output to target/examples/plug_hello/.
(
  cd examples/plug_hello
  mix local.hex --force --if-missing
  mix local.rebar --force --if-missing
  mix deps.get --check-locked
  MIX_ENV='test' mix compile --warnings-as-errors
  mix test --cover --warnings-as-errors
  MIX_ENV=prod mix release --overwrite
)
# The bcrypt fixture fetches bcrypt_elixir from Hex and builds its NIF library with the host cc. Its entry module has the same gates: a compile with warnings as errors and a test with 100% coverage.
(
  cd fixtures/bcrypt
  mix deps.get --check-locked
  MIX_ENV='test' mix compile --warnings-as-errors
  mix test --cover --warnings-as-errors
)
# The exqlite fixture fetches exqlite from Hex and builds its NIF library from source with the host cc (config :exqlite, force_build: true). Its entry module has the same gates.
(
  cd fixtures/exqlite
  mix deps.get --check-locked
  MIX_ENV='test' mix compile --warnings-as-errors
  mix test --cover --warnings-as-errors
)
# The fs_notify fixture fetches fs_notify from Hex and builds its NIF library from source with rustler and the host cargo (config :rustler_precompiled, force_build_all: true). Its entry module has the same gates.
(
  cd fixtures/fs_notify
  mix deps.get --check-locked
  MIX_ENV='test' mix compile --warnings-as-errors
  mix test --cover --warnings-as-errors
)
