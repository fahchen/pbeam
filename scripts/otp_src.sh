#!/usr/bin/env bash
# Print lines <start> to <end> of an OTP source file at the pinned OTP tag, with line numbers.
set -euo pipefail

path=$1 start=$2 end=$3
if ! [[ $start =~ ^[1-9][0-9]*$ && $end =~ ^[1-9][0-9]*$ ]] || ((start > end)); then
  echo "otp-src: $start and $end must be line numbers with start <= end" >&2
  exit 1
fi

tag="OTP-$(mise config get --file mise.toml tools.erlang)"
src=${PBEAM_OTP_SRC:-$HOME/.cache/pbeam/otp}
if ! git -C "$src" rev-parse --git-dir >/dev/null 2>&1; then
  echo "otp-src: no OTP git repository at $src" >&2
  echo 'Set PBEAM_OTP_SRC to an erlang/otp clone, or run:' >&2
  echo "git clone --bare --depth 1 --branch $tag https://github.com/erlang/otp.git $src" >&2
  exit 1
fi
if ! git -C "$src" rev-parse --verify --quiet "refs/tags/$tag" >/dev/null; then
  # A --depth fetch makes a full clone shallow, so give it only to a shallow repository.
  depth=
  [[ $(git -C "$src" rev-parse --is-shallow-repository) == true ]] && depth=' --depth 1'
  echo "otp-src: no tag $tag in $src; run:" >&2
  echo "git -C $src fetch$depth origin tag $tag" >&2
  exit 1
fi

# Read the blob before awk: awk stops at <end>, and a pipe from git would then fail with SIGPIPE.
# The `.` keeps trailing blank lines; the here-string adds back the last newline.
blob=$(git -C "$src" cat-file blob "$tag:$path" && echo .)
blob=${blob%.}
awk -v head="$tag $path:$start-$end" -v start="$start" -v end="$end" -v path="$path" '
  NR == start { print head }
  NR >= start { print NR "\t" $0 }
  NR == end { exit }
  END { if (NR < start) { print "otp-src: " path " has " NR " lines" > "/dev/stderr"; exit 1 } }' <<<"${blob%$'\n'}"
