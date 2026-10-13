#!/usr/bin/env bash
# Measure the peak live bytes of one Terra scene of task #587 and write it to
# a file. The measurement is the integration test
# tests/integration/term_live_bytes.rs on a release build, because the record
# numbers of the term work use the release profile. The test loads one scene
# of the Terra Counter target and samples the bytes of the guest containers
# at each scheduling step. The file holds the revision, the host, the
# toolchain, the load, and the report of the run: the peak live bytes, the
# bytes added and released with the counts of both, the spread of a second
# run, and the peak resident set size of the test process.
#
# Usage: bash scripts/live_bytes.sh [output file]
#
# The default output file is target/live-bytes.txt. The task prints the same
# text.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain, as mise sets it: a plain PATH can hold another Rust
# release, and the record must name the tools that the numbers use.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

out=${1:-target/live-bytes.txt}
mkdir -p "$(dirname "$out")"

{
  printf '== term live bytes ==\n'
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

# One harness thread, so a second test process does not compete with the
# measured run. The number of the live bytes does not change with the load,
# and the resident set size sample does.
cargo test --locked --release --test term_live_bytes \
  records_the_peak_live_bytes_of_one_terra_scene -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
