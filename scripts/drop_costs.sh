#!/usr/bin/env bash
# Measure the drop cost of a term of docs/interpreter-dispatch-plan.md step 7
# and write it to a file. The benchmark is tests/integration/drop_costs.rs,
# and the release profile holds the numbers that the plan uses. Each number is
# the minimum of the timed runs of one shape, in nanosecond for each term.
#
# Usage: bash scripts/drop_costs.sh [output file]
#
# The default output file is target/drop-costs.txt. The file holds the
# revision, the host, the toolchain, and the table of the benchmark. The task
# prints the same text.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain, as mise sets it: a plain PATH can hold another Rust
# release, and the record must name the tools that the numbers use.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

out=${1:-target/drop-costs.txt}
mkdir -p "$(dirname "$out")"

{
  printf '== term drop costs ==\n'
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

# One harness thread, so that the shapes of the table do not compete for the
# CPU and the caches of the host.
cargo test --locked --release --test drop_costs records_the_drop_cost_of_a_term -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
