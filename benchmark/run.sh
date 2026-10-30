#!/usr/bin/env bash
# Compile the scenarios of benchmark/ with the pinned erlc and compare the PBeam
# runtime with the pinned ERTS. The comparison is benchmark/benchmark.rs, which
# runs each scenario through the library API and through `erl +S 1 -noshell`.
#
# Usage: bash benchmark/run.sh
#
# The script prints the revision, the host, the toolchain, the load, and the
# comparison table of benchmark/benchmark.rs.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain, as mise sets it: a plain PATH can hold another OTP or
# Rust release, and the record must name the tools that the numbers use.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

beams=target/benchmark/beams
mkdir -p "$beams"

{
  printf '== PBeam and ERTS benchmark ==\n'
  printf 'revision %s\n' "$(git rev-parse HEAD)"
  # A change in the working tree makes the record exact, because the measured
  # code is then not the commit.
  git diff --quiet HEAD 2>/dev/null || printf 'the working tree differs from HEAD\n'
  printf 'host %s, %s\n' \
    "$(uname -srm)" \
    "$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown)"
  printf 'cpus %s\n' "$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo unknown)"
  printf 'load %s\n' \
    "$(sysctl -n vm.loadavg 2>/dev/null || cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || echo unknown)"
  printf 'rustc %s\n' "$(rustc --version)"
  printf 'cargo %s\n' "$(cargo --version)"
  erl -noshell -eval 'io:format("erlc OTP ~s, ERTS ~s, ~s~n", [erlang:system_info(otp_release), erlang:system_info(version), erlang:system_info(emu_flavor)]), halt().'
  printf '\n'
}

erlc +deterministic -o "$beams" benchmark/bench_arith.erl benchmark/bench_spawn.erl benchmark/bench_list.erl
cargo run --quiet --locked --release --example benchmark
