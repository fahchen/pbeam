#!/usr/bin/env bash
# Measure the interpreter dispatch baseline of docs/interpreter-dispatch-plan.md
# and write it to a file. The benchmark is tests/integration/dispatch_baseline.rs,
# and the release profile holds the numbers that the plan uses. Each number is
# the minimum of nine runs of one probe. The layout of the decoded form and its
# bytes for each iteration are library unit tests, because the run loop keeps
# them private; this script runs those tests too.
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
  # The cache sizes are host facts of the step 6 measurement. macOS reports
  # the efficiency core values in the plain keys and the performance core
  # values in the `perflevel` keys.
  printf 'caches l1i %s, l1d %s, l2 %s, line %s\n' \
    "$(sysctl -n hw.l1icachesize 2>/dev/null || echo unknown)" \
    "$(sysctl -n hw.l1dcachesize 2>/dev/null || echo unknown)" \
    "$(sysctl -n hw.l2cachesize 2>/dev/null || echo unknown)" \
    "$(sysctl -n hw.cachelinesize 2>/dev/null || echo unknown)"
  if sysctl -n hw.perflevel0.l1icachesize >/dev/null 2>&1; then
    printf 'caches performance cores l1i %s, l1d %s, l2 %s\n' \
      "$(sysctl -n hw.perflevel0.l1icachesize)" \
      "$(sysctl -n hw.perflevel0.l1dcachesize)" \
      "$(sysctl -n hw.perflevel0.l2cachesize)"
    printf 'caches efficiency cores l1i %s, l1d %s, l2 %s\n' \
      "$(sysctl -n hw.perflevel1.l1icachesize)" \
      "$(sysctl -n hw.perflevel1.l1dcachesize)" \
      "$(sysctl -n hw.perflevel1.l2cachesize)"
  fi
  printf 'rustc %s\n' "$(rustc --version)"
  printf 'cargo %s\n' "$(cargo --version)"
  erl -noshell -eval 'io:format("erts OTP ~s, erts ~s, ~s, schedulers ~b, logical processors ~b~n", [erlang:system_info(otp_release), erlang:system_info(version), erlang:system_info(emu_flavor), erlang:system_info(schedulers), erlang:system_info(logical_processors)]), halt().'
  printf '\n'
} | tee "$out"

# The decoded form of the run loop is not public, so its layout and its
# per-iteration bytes are unit tests of the library.
cargo test --locked --release --lib records_the_decoded -- --nocapture --test-threads=1 2>&1 | tee -a "$out"

# The cache proxy and the dispatch benchmark compete for the CPU and the
# caches when they run together, and both numbers are then weaker. Run one
# test for each command, and one thread for each harness.
cargo test --locked --release --test dispatch_baseline records_the_instruction_stream_cache_proxy -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
cargo test --locked --release --test dispatch_baseline records_the_interpreter_dispatch_baseline -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
cargo test --locked --release --test dispatch_baseline keeps_the_measured_instructions_in_each_loop -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
