#!/usr/bin/env bash
# Run the Terra Counter reference scenario of docs/terra-reference.md on the pinned ERTS in a pseudo-terminal, then run it again with the ERTS call counters on and write the call inventory.
# Usage: bash scripts/terra_reference.sh (or mise run terra-reference). macOS only: the pseudo-terminal comes from the macOS script(1) utility.
# The first run is the command of examples/terra_counter/README.md, `bash scripts/example.sh terra_counter --erts`. It builds the example and runs it. The build downloads the pinned Terra commit or uses PBEAM_TERRA_SRC (scripts/terra.sh), and the Elixir compiler runs the macros of the Terra sources.
# The second run starts erl with the code path of the build (target/examples/terra_counter/erts.paths) and runs TerraCounter.main/0 through terra_calls:run/2 (scripts/terra_calls.erl).
# Output in target/reference/: versions.txt, run/report.txt, inventory/report.txt, inventory/calls.eterm, and inventory.txt. Exit status: 0 when both runs pass each check of scripts/terra_reference.escript, 1 when a check fails, 2 for a setup error.
set -euo pipefail

fail() {
  echo "terra-reference: $1" >&2
  exit 2
}

[[ $(uname -s) == Darwin ]] || fail 'the pseudo-terminal harness uses the macOS script(1) utility; run it on macOS'
[[ -f scripts/terra_calls.erl ]] || fail 'run the script from the repository root'
out=target/reference
example=target/examples/terra_counter
rm -rf "$out"
mkdir -p "$out/ebin"

status=0
escript scripts/terra_reference.escript "$out/run" -- bash scripts/example.sh terra_counter --erts || status=1

terra=$(bash scripts/terra.sh)
{
  echo "terra: $(git -C "$terra" rev-parse HEAD), mix.lock blob $(git -C "$terra" rev-parse HEAD:mix.lock)"
  echo "erl: $(command -v erl)"
  erl -noshell -eval 'io:format("otp ~s, erts ~s~n", [erlang:system_info(otp_release), erlang:system_info(version)]), halt().'
  elixir --version | tail -n 1
  rustc --version
  echo "macOS $(sw_vers -productVersion) ($(sw_vers -buildVersion)), $(uname -m)"
} >"$out/versions.txt"
cat "$out/versions.txt"

erlc -o "$out/ebin" scripts/terra_calls.erl
paths=()
while IFS= read -r path; do
  paths+=(-pa "$path")
done <"$example/erts.paths"
escript scripts/terra_reference.escript "$out/inventory" -- erl +S 1 -noshell "${paths[@]}" -pa "$out/ebin" -eval "terra_calls:run(\"$example/pbeam.beams\", \"$out/inventory/calls.eterm\"), halt()." || status=1
if [[ -f $out/inventory/calls.eterm ]]; then
  escript scripts/terra_inventory.escript "$out/inventory/calls.eterm" "$example/pbeam.beams" >"$out/inventory.txt"
  echo "terra-reference: call inventory in $out/inventory.txt"
fi
exit "$status"
