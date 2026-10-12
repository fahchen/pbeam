#!/usr/bin/env bash
# Measure the instruction attribution of task #546 and write it to a file.
#
# The record is docs/interpreter-dispatch-measurements.md. The method is an
# ablation: a scratch build neutralises one sub-step of the interpreter, and
# this script measures the same floor loop in that build. The base build gives
# the reference points, and the difference of a variant and the base build is
# the share of the neutralised sub-step.
#
# The script also runs the micro-measurement of the parts that an ablation
# cannot separate, at the same revision. That measurement reads the private
# helpers of the interpreter, so it is a library unit test.
#
# The release profile holds the numbers. Run one script call for one build,
# and give each call its variant label. The file records the revision, the
# label, the clock, the JST time, the host, and the load, so the record of the
# pull request names the conditions of every run.
#
# Usage: bash scripts/dispatch_attribution.sh [label] [output file]
#
# The default label is `base`. The default output file is
# target/dispatch-attribution.txt.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain, as mise sets it: a plain PATH can hold another Rust
# release, and the record must name the tools that the numbers use.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

label=${1:-base}
out=${2:-target/dispatch-attribution.txt}
mkdir -p "$(dirname "$out")"

{
  printf '== interpreter instruction attribution ==\n'
  printf 'variant %s\n' "$label"
  printf 'revision %s\n' "$(git rev-parse HEAD)"
  # A change in the working tree makes the record exact, because the measured
  # code is then not the commit. A scratch ablation variant reports this line.
  git diff --quiet HEAD 2>/dev/null || printf 'the working tree differs from HEAD\n'
  printf 'clock %s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
  printf 'jst %s\n' "$(TZ=Asia/Tokyo date '+%Y-%m-%d %H:%M:%S %Z')"
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

# The measured loop must hold the instructions that the attribution names. A
# change of the compiled shape fails here, because the reference points would
# then measure other work.
cargo test --locked --release --test dispatch_baseline keeps_the_measured_instructions_in_each_loop \
  -- --nocapture --test-threads=1 2>&1 | tee -a "$out"

# The two reference points of the floor loop. One harness thread, so the
# probe does not compete with the other probes of the target.
cargo test --locked --release --test dispatch_baseline records_the_attribution_reference_points \
  -- --nocapture --test-threads=1 2>&1 | tee -a "$out"

# The parts that an ablation cannot separate. The same build, one thread.
cargo test --locked --release --lib dispatch_attribution_costs \
  -- --nocapture --test-threads=1 2>&1 | tee -a "$out"
