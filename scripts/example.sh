#!/usr/bin/env bash
# Build and run the example examples/<name> on the pbeam binary, or on the pinned ERTS with --erts.
# Usage: bash scripts/example.sh <name> [--erts]
# The example is the Elixir sources examples/<name>/*.ex. The entry is main/0 of the module with the camel-case name of the example, for example HelloWorld for hello_world.
# The compiled example modules and pbeam_boot.beam go into target/examples/<name>/.
# PBEAM_BIN selects the pbeam binary. Without it, the script runs the binary with cargo run.
# The commands are those of the Run section of examples/hello_world/README.md.
set -euo pipefail

fail() {
  echo "example: $1" >&2
  exit 2
}

[[ $# -ge 1 && $# -le 2 ]] || fail 'usage: example <name> [--erts]'
name=$1
erts=false
if [[ $# -eq 2 ]]; then
  [[ $2 == --erts ]] || fail "unknown option $2"
  erts=true
fi
[[ $name =~ ^[a-z][a-z0-9_]*$ ]] || fail "the name $name is not a lowercase name such as hello_world"
sources=(examples/"$name"/*.ex)
[[ -e ${sources[0]} ]] || fail "examples/$name has no .ex file"

entry=Elixir.
IFS=_ read -ra words <<<"$name"
for word in "${words[@]}"; do
  entry+=$(tr '[:lower:]' '[:upper:]' <<<"${word:0:1}")${word:1}
done

# The build tools elixirc and elixir start the elixir application, which writes a warning to stderr when the file name encoding is latin1. ERTS on Linux selects latin1 in the C locale. The example files have ASCII names, so the build uses UTF-8 file names in each locale. The erl runs of the example are not changed.
export ELIXIR_ERL_OPTIONS="${ELIXIR_ERL_OPTIONS:-} +fnu"

# Compile into a directory of this run, then rename each file into the output directory. A rename replaces a file atomically, so a run or a test that reads a file at the same time reads a complete file. erlc deletes its output file first and writes through the fixed temporary file pbeam_boot.bea# (compile.erl remove_file/2 and save_binary_1/2), so two compilations into one directory can collide.
output=target/examples/$name
mkdir -p "$output"
work=$(mktemp -d "$output/compile.XXXXXX")
trap 'rm -rf "$work"' EXIT
elixirc --warnings-as-errors -o "$work" "${sources[@]}"
[[ -f $work/$entry.beam ]] || fail "examples/$name defines no module ${entry#Elixir.}"
erlc +deterministic -o "$work" examples/hello_world/pbeam_boot.erl
examples=()
for beam in "$work"/*.beam; do
  file=${beam##*/}
  mv -f "$beam" "$output/$file"
  [[ $file == pbeam_boot.beam ]] || examples+=("$output/$file")
done
rm -rf "$work"
trap - EXIT

# A command substitution, not a process substitution, so that set -e stops the run when the command fails.
elixir_list=$(elixir scripts/example_beams.exs "${examples[@]}")
elixir_beams=()
while IFS= read -r beam; do
  if [[ -n $beam ]]; then elixir_beams+=("$beam"); fi
done <<<"$elixir_list"

if [[ $erts == true ]]; then
  paths=(-pa "$output")
  for beam in ${elixir_beams[@]+"${elixir_beams[@]}"}; do
    case " ${paths[*]} " in
      *" ${beam%/*} "*) ;;
      *) paths+=(-pa "${beam%/*}") ;;
    esac
  done
  exec erl -noshell "${paths[@]}" -eval "'$entry':main(), halt()."
fi

boot_modules="erlang prim_tty proc_lib application application_controller lists os unicode gen_event gen prim_tty_sighandler erl_signal_handler standard_error supervisor_bridge user_sup user_drv group gen_statem io io_lib io_lib_format proplists queue gen_server logger logger_config edlin edlin_key logger_server logger_simple_h logger_sup supervisor logger_std_h logger_h_common logger_olp logger_proxy logger_handler_watcher logger_formatter maps init erts_internal"
# shellcheck disable=SC2086 # Each module name is one argument.
boot_list=$(erl -noshell -eval '[io:format("~ts~n", [case code:which(list_to_atom(M)) of preloaded -> filename:join([code:lib_dir(erts), "ebin", M ++ ".beam"]); Path -> Path end]) || M <- init:get_plain_arguments()], halt().' -extra $boot_modules)
boot_beams=()
while IFS= read -r beam; do
  boot_beams+=("$beam")
done <<<"$boot_list"

beams=("${examples[@]}" ${elixir_beams[@]+"${elixir_beams[@]}"} "${boot_beams[@]}" "$output/pbeam_boot.beam")
if [[ -n ${PBEAM_BIN:-} ]]; then
  exec "$PBEAM_BIN" pbeam_boot start "$entry" main -- "${beams[@]}"
fi
exec cargo run --quiet --locked --bin pbeam -- pbeam_boot start "$entry" main -- "${beams[@]}"
