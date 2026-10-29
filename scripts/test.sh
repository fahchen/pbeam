#!/usr/bin/env bash
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

# Only the exact empty bootstrap is exempt; any Rust source addition enables coverage.
rust_sources=$(git ls-files --cached --others --exclude-standard -- '*.rs')
if [[ "$rust_sources" == 'src/lib.rs' ]] &&
  [[ "$(cat src/lib.rs)" == '//! Portable BEAM runtime. Implementation begins in subsequent task PRs.' ]]; then
  cargo test --locked --workspace --all-features
  echo 'Line coverage: N/A (empty bootstrap; no executable Rust code).'
else
  # nextest runs each test in its own process, in parallel across the test binaries.
  # The slow status level prints only slow and failed tests, not a line for each passed test.
  # --no-fail-fast runs all tests after a failure, so the output shows each failure.
  # By default, cargo-llvm-cov runs `cargo clean -p pbeam-runtime`, which removes the incremental state of the crate.
  # --no-clean keeps that state, so rustc reuses the unchanged code of the last local run.
  # The profile files of the last run must go, or their counts would cover lines that no test runs now.
  # CARGO_TARGET_DIR or a cargo config can move the coverage directory.
  # The pin makes cargo-llvm-cov use the directory that the cleanup below removes files from.
  cov_dir=target/llvm-cov-target
  export CARGO_LLVM_COV_TARGET_DIR=$cov_dir
  cargo llvm-cov clean --profraw-only
  rm -f "$cov_dir"/*.profdata
  # A change of the profile or the features gives the test executables new file names, and the old executables stay.
  # cargo-llvm-cov reports each executable whose name matches a workspace target, so an old executable would add uncovered lines.
  # Each executable of the workspace has no extension; dependency files such as proc-macro libraries have one and stay.
  # rustc links the executables again from the incremental state.
  deps=$cov_dir/debug/deps
  if [[ -d "$deps" ]]; then
    find "$deps" -maxdepth 1 -type f -perm -u+x ! -name '*.*' -delete
  fi
  cargo llvm-cov nextest --locked --workspace --all-features --no-clean \
    --status-level slow --final-status-level slow --no-fail-fast \
    --cobertura --output-path target/coverage.xml
  mise exec -- elixir scripts/check_coverage.exs target/coverage.xml
fi
