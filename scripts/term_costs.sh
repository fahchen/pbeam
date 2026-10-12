#!/usr/bin/env bash
# Measure the cost of one term operation of
# docs/interpreter-dispatch-measurements.md and write it to a file. The
# measurement is the library unit tests `prints_the_cost_of_a_term_operation`,
# `prints_the_cost_of_a_fun_payload`, and `prints_the_cost_of_a_map_payload`,
# because the payload types of the fun and of the map are private to their
# modules. Each printed number is the minimum of nine timed runs of one
# operation, and the release profile holds the report numbers.
#
# Usage: bash scripts/term_costs.sh [output file]
#
# The default output file is target/term-costs.txt. The file holds the
# revision, the host, the toolchain, the load, and one line for each measured
# operation. The task prints the same text.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain, as mise sets it: a plain PATH can hold another Rust
# release, and the record must name the tool that the numbers use.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

out=${1:-target/term-costs.txt}
mkdir -p "$(dirname "$out")"

{
  printf '== term operation costs ==\n'
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
  printf '\n'
} | tee "$out"

# One harness thread, so the measured operation does not compete with the
# other tests of the binary for the CPU.
cargo test --locked --release --lib prints_the_cost_of_a_ -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
