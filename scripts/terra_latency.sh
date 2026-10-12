#!/usr/bin/env bash
# Measure the Terra Counter start time and key latency of the pbeam binary, and
# compare it with pinned ERTS. One command does everything: it builds the
# example and the binaries when they are missing, then measures each mode.
#
# Run this in a terminal. The example needs a pseudo-terminal, so a sandbox that
# denies /dev/ptmx cannot run it.
#
# Usage: bash scripts/terra_latency.sh [--rebuild] [debug|release|erts|all] [output file]
#
# The default mode is all, and the default output file is
# target/terra-latency.txt. Each run appends one block with its label, the
# revision, the host load, and the measured times.
set -euo pipefail

cd "$(dirname "$0")/.."

# The pinned toolchain: the example script calls erl, elixir, and elixirc, and a
# plain PATH can hold another OTP release, which gives the binary a wrong root.
export LANG=en_US.UTF-8
export LC_ALL=en_US.UTF-8

rebuild=false
if [[ ${1:-} == --rebuild ]]; then
  rebuild=true
  shift
fi

mode=${1:-all}
out=${2:-target/terra-latency.txt}
# The revision of the measured code. A change in the working tree makes the
# record exact, because the measured code is then not the commit.
revision=$(git rev-parse --short HEAD)
git diff --quiet HEAD 2>/dev/null || revision="$revision+dirty"

case "$mode" in
  debug | release | erts | all) ;;
  *)
    echo "usage: bash scripts/terra_latency.sh [--rebuild] [debug|release|erts|all] [output file]" >&2
    exit 2
    ;;
esac

build_binary() {
  local profile=$1
  shift
  if [[ $rebuild == true || ! -x "target/$profile/pbeam" ]]; then
    echo "==> build the $profile pbeam binary"
    mise exec -- cargo build --locked "$@"
  fi
}

# A run with --no-build reads the code path of the ERTS run and the start set of
# the pbeam run, and the build renames each list of the last build into place
# last. Thus the two lists together mean a complete build. The example build
# runs before the first frame starts, so a run that cannot open a terminal still
# leaves a complete build.
example=target/examples/terra_counter
if [[ ! -f $example/pbeam.start || ! -f $example/erts.paths ]]; then
  echo "==> build the terra_counter example"
  mise run example terra_counter --erts </dev/null >/dev/null 2>&1 || true
  if [[ ! -f $example/pbeam.start || ! -f $example/erts.paths ]]; then
    echo "the example build failed. Run it by hand: LANG=en_US.UTF-8 mise run example terra_counter --erts" >&2
    exit 2
  fi
fi

mkdir -p "$(dirname "$out")"

measure() {
  local label=$1
  shift
  {
    printf '\n== %s ==\n' "$label"
    printf 'revision %s, %s, host load %s\n' "$revision" "$(date '+%Y-%m-%d %H:%M:%S')" \
      "$(sysctl -n vm.loadavg 2>/dev/null || cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || echo unknown)"
  } | tee -a "$out"
  # A measurement that fails, for example because the program never draws its
  # first frame, must not stop the modes that follow it.
  python3 -u scripts/terra_latency.py "$@" 2>&1 | tee -a "$out" || true
}

if [[ $mode == debug || $mode == all ]]; then
  build_binary debug
  measure "debug build" mise exec -- env PBEAM_BIN="$PWD/target/debug/pbeam" \
    bash scripts/example.sh terra_counter --no-build
fi

if [[ $mode == release || $mode == all ]]; then
  build_binary release --release
  measure "release build" mise exec -- env PBEAM_BIN="$PWD/target/release/pbeam" \
    bash scripts/example.sh terra_counter --no-build
fi

if [[ $mode == erts || $mode == all ]]; then
  measure "pinned ERTS, erl +S 1" mise exec -- \
    bash scripts/example.sh terra_counter --erts --no-build
fi

printf '\nthe measurements are in %s\n' "$out"
