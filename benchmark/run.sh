#!/usr/bin/env bash
# Compare the PBeam runtime with the pinned ERTS on the scenarios of benchmark/.
# The comparison is tests/integration/benchmark.rs. It compiles benchmark/*.erl
# with the pinned erlc, runs each scenario through the library API and through
# `erl +S 1 -noshell`, and prints the table.
#
# Usage: bash benchmark/run.sh
#
# The script prints the revision, the host, the toolchain, the load, and the
# comparison table of tests/integration/benchmark.rs.
set -euo pipefail

cd "$(dirname "$0")/.."

# A plain PATH can hold another OTP or Rust release, so each tool runs through
# `mise exec` with the pins of mise.toml, also when this script runs without
# `mise run`. The record names the tools that the numbers use.
pinned() { mise exec -- "$@"; }
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

{
  printf '== PBeam and ERTS benchmark ==\n'
  printf 'revision %s\n' "$(git rev-parse HEAD)"
  # A change in the working tree makes the record inexact, because the measured
  # code is then not the commit.
  git diff --quiet HEAD 2>/dev/null || printf 'the working tree differs from HEAD\n'
  printf 'host %s, %s\n' \
    "$(uname -srm)" \
    "$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)"
  printf 'cpus %s\n' "$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo unknown)"
  printf 'load %s\n' \
    "$(sysctl -n vm.loadavg 2>/dev/null || cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || echo unknown)"
  printf 'rustc %s\n' "$(pinned rustc --version)"
  printf 'cargo %s\n' "$(pinned cargo --version)"
  pinned erl -noshell -eval 'io:format("erlc OTP ~s, ERTS ~s, ~s~n", [erlang:system_info(otp_release), erlang:system_info(version), erlang:system_info(emu_flavor)]), halt().'
  printf '\n'
}

pinned cargo test --locked --release --test benchmark -- --nocapture
