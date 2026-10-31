#!/usr/bin/env bash
set -euo pipefail

mode=${PBEAM_FIXTURE_MODE:-full}
if [[ "$mode" != full && "$mode" != prepare ]]; then
  echo "unknown PBEAM_FIXTURE_MODE: $mode" >&2
  exit 1
fi

if [[ "$mode" == full ]]; then
  # The root Mix project holds PBeam.Native and requires 100% coverage.
  mix test --cover --warnings-as-errors
  (cd fixtures/hello && mix test --cover --warnings-as-errors)
else
  # Rust tests read the test-profile hello BEAM files in each partition.
  (cd fixtures/hello && MIX_ENV='test' mix compile --warnings-as-errors)
fi

# Rust tests use the Plug release in each partition. Full checks also verify
# the Plug test profile and its application line coverage.
(
  cd examples/plug_hello
  mix local.hex --force --if-missing
  mix local.rebar --force --if-missing
  mix deps.get --check-locked
  if [[ "$mode" == full ]]; then
    MIX_ENV='test' mix compile --warnings-as-errors
    mix test --cover --warnings-as-errors
  fi
  MIX_ENV=prod mix release --overwrite
)

if [[ "$mode" == full ]]; then
  # Each additional fixture requires warnings-as-errors and 100% coverage.
  (
    cd fixtures/bcrypt
    mix deps.get --check-locked
    MIX_ENV='test' mix compile --warnings-as-errors
    mix test --cover --warnings-as-errors
  )
  (
    cd fixtures/exqlite
    mix deps.get --check-locked
    MIX_ENV='test' mix compile --warnings-as-errors
    mix test --cover --warnings-as-errors
  )
  (
    cd fixtures/fs_notify
    mix deps.get --check-locked
    MIX_ENV='test' mix compile --warnings-as-errors
    mix test --cover --warnings-as-errors
  )
fi
