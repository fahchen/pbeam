#!/usr/bin/env bash
# Measure the interpreter dispatch baseline of docs/interpreter-dispatch-plan.md
# and write it to a file. The benchmark is tests/integration/dispatch_baseline.rs,
# and the release profile holds the numbers that the plan uses. Each number is
# the minimum of nine runs of one probe.
#
# Usage: bash scripts/dispatch_baseline.sh [output file]
#
# The default output file is target/dispatch-baseline.txt. The file holds the
# revision, the host, the toolchain, the pinned ERTS build, and the tables of
# the benchmark. The task prints the same text.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain, as mise sets it: a plain PATH can hold another OTP or
# Rust release, and the record must name the tools that the numbers use.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

out=${1:-target/dispatch-baseline.txt}
mkdir -p "$(dirname "$out")"

{
  printf '== interpreter dispatch baseline ==\n'
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
  erl -noshell -eval 'io:format("erts OTP ~s, erts ~s, ~s, schedulers ~b, logical processors ~b~n", [erlang:system_info(otp_release), erlang:system_info(version), erlang:system_info(emu_flavor), erlang:system_info(schedulers), erlang:system_info(logical_processors)]), halt().'
  printf '\n'
} | tee "$out"

cargo test --locked --release --test dispatch_baseline -- --nocapture 2>&1 | tee -a "$out"
