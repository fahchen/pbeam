#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

case $#:${1:-} in
  0:|1:--write) ;;
  *)
    echo 'Usage: bash scripts/otp_suite_beam.sh [--write]' >&2
    exit 2
    ;;
esac

otp=vendor/otp
if ! pin=$(git rev-parse --verify ":$otp" 2>/dev/null) ||
   [[ $(git -C "$otp" rev-parse HEAD 2>/dev/null) != "$pin" ]]; then
  echo "otp-suite-beam: check out the pinned source with git submodule update --init $otp" >&2
  exit 1
fi

version=$(mise config get --file mise.toml tools.erlang)
otp_root=$(erl -noshell -noinput -eval 'io:put_chars(code:root_dir()), halt().')
if [[ $(git -C "$otp" show "$pin:OTP_VERSION") != "$version" ||
      $(cat "$otp_root/releases/${version%%.*}/OTP_VERSION" 2>/dev/null) != "$version" ]]; then
  echo "otp-suite-beam: both the source and the Erlang installation must be OTP-$version" >&2
  exit 1
fi

sources=(
  erts/emulator/test/bs_match_int_SUITE.erl
  erts/emulator/test/binary_SUITE.erl
  lib/stdlib/test/lists_SUITE.erl
)
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT
for source in "${sources[@]}"; do
  git -C "$otp" show "$pin:$source" > "$scratch/${source##*/}"
done
env -u ERL_COMPILER_OPTIONS "$otp_root/bin/erlc" -W0 +deterministic +no_debug_info -o "$scratch" "$scratch"/*.erl

status=0
for source in "${sources[@]}"; do
  beam=${source%.erl}.beam
  output=tests/otp/beam/$beam
  if [[ ${1:-} == --write ]]; then
    mkdir -p "$(dirname "$output")"
    cp "$scratch/${beam##*/}" "$output"
  elif ! cmp -s "$scratch/${beam##*/}" "$output"; then
    echo "otp-suite-beam: $output differs from OTP-$version; regenerate with --write" >&2
    status=1
  fi
done
if ! diff -u \
  <(printf '%s\n' "${sources[@]/%.erl/.beam}" | LC_ALL=C sort) \
  <(cd tests/otp/beam && find . -name '*.beam' | sed 's|^\./||' | LC_ALL=C sort); then
  echo 'otp-suite-beam: committed BEAM paths must match the upstream source list' >&2
  status=1
fi
exit "$status"
