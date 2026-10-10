#!/usr/bin/env bash
# Build and run the example examples/<name> on the pbeam binary, or on the pinned ERTS with --erts.
# The ERTS run uses one scheduler (+S 1), because the pbeam binary runs all processes on one scheduler thread. With more schedulers, ERTS can run two busy processes at the same time, and the order of their output changes from run to run.
# Usage: bash scripts/example.sh <name> [--erts] [--no-build] [--policy <module>]
# The example is the Elixir sources examples/<name>/*.ex. The entry is main/0 of the module with the camel-case name of the example, for example HelloWorld for hello_world.
# The script has two steps, the build and the run. The build writes all the files that a run needs into target/examples/<name>/. The run starts only erl or the pbeam binary. With --no-build, the script does not build: it reads a list of the last build and runs it. Thus the tests build each example once, and the ERTS run and the pbeam run execute the same compiled modules.
# The build compiles the example modules and pbeam_boot.beam into target/examples/<name>/, and consolidates the Elixir protocols into target/examples/<name>/consolidated/, as `mix run` consolidates them (scripts/example_beams.exs). It finds each OTP and Elixir module that the example calls, directly or through other modules (scripts/example_beams.exs). It then writes the code path of the ERTS run into target/examples/<name>/erts.paths and the BEAM files of the pbeam run into target/examples/<name>/pbeam.beams, one for each line. It writes each called module that the pbeam run does not get, with the reason, into target/examples/<name>/pbeam.unloaded.
# The terra_counter example also compiles the Terra sources of the pinned commit, which scripts/terra.sh downloads or finds in PBEAM_TERRA_SRC.
# With --policy <module>, the pbeam run gives --policy <module> to the pbeam binary, so the policy module of the build gives the capabilities of the run (docs/capability-security-migration.md, section "Decisions of #338"). The ERTS run has no policy, so the script rejects --policy with --erts.
# PBEAM_BIN selects the pbeam binary. Without it, the script runs the binary with cargo run.
# The commands are those of the Run section of examples/hello_world/README.md.
set -euo pipefail

fail() {
  echo "example: $1" >&2
  exit 2
}

# Print a key step to stderr only when stderr is a terminal. A run with stderr in a pipe, as in the tests, keeps the stderr of the program byte-identical with ERTS.
step() {
  if [[ -t 2 ]]; then echo "==> $1" >&2; fi
}

usage='usage: example <name> [--erts] [--no-build] [--policy <module>]'
[[ $# -ge 1 ]] || fail "$usage"
name=$1
shift
erts=false
build=true
policy=()
while [[ $# -gt 0 ]]; do
  case $1 in
    --erts) erts=true ;;
    --no-build) build=false ;;
    --policy)
      [[ $# -ge 2 ]] || fail "$usage"
      policy=(--policy "$2")
      shift
      ;;
    *) fail "unknown option $1" ;;
  esac
  shift
done
[[ $erts == false || ${#policy[@]} -eq 0 ]] || fail 'the ERTS run has no policy: give --policy without --erts'
[[ $name =~ ^[a-z][a-z0-9_]*$ ]] || fail "the name $name is not a lowercase name such as hello_world"
sources=(examples/"$name"/*.ex)
[[ -e ${sources[0]} ]] || fail "examples/$name has no .ex file"

entry=Elixir.
IFS=_ read -ra words <<<"$name"
for word in "${words[@]}"; do
  entry+=$(tr '[:lower:]' '[:upper:]' <<<"${word:0:1}")${word:1}
done
output=target/examples/$name
consolidated=$output/consolidated

# Run the example on the pinned ERTS with the code path in the arguments, or on the pbeam binary with the BEAM files in the arguments. The function does not return.
run() {
  if [[ $erts == true ]]; then
    local paths=()
    for path in "$@"; do
      paths+=(-pa "$path")
    done
    step "Run ${entry#Elixir.}.main/0 on the pinned ERTS (erl +S 1)"
    exec erl +S 1 -noshell "${paths[@]}" -eval "'$entry':main(), halt()."
  fi
  if [[ -n ${PBEAM_BIN:-} ]]; then
    step "Run ${entry#Elixir.}.main/0 on $PBEAM_BIN with $# BEAM files"
    exec "$PBEAM_BIN" ${policy[@]+"${policy[@]}"} pbeam_boot start "$entry" main -- "$@"
  fi
  step "Build pbeam and run ${entry#Elixir.}.main/0 with $# BEAM files"
  exec cargo run --quiet --locked --bin pbeam -- ${policy[@]+"${policy[@]}"} pbeam_boot start "$entry" main -- "$@"
}

# With --no-build, run the last build with the list that it wrote.
if [[ $build == false ]]; then
  list=$output/pbeam.beams
  if [[ $erts == true ]]; then list=$output/erts.paths; fi
  [[ -f $list ]] || fail "$list does not exist. Build the example first: run the script without --no-build"
  items=()
  while IFS= read -r item; do
    items+=("$item")
  done <"$list"
  run "${items[@]}"
fi

# The build tools elixirc and elixir start the elixir application, which writes a warning to stderr when the file name encoding is latin1. ERTS on Linux selects latin1 in the C locale. The example files have ASCII names, so the build uses UTF-8 file names in each locale. The erl runs of the example are not changed.
export ELIXIR_ERL_OPTIONS="${ELIXIR_ERL_OPTIONS:-} +fnu"

# Compile into a directory of this run, then rename each file into the output directory. A rename replaces a file atomically, so a run or a test that reads a file at the same time reads a complete file. erlc deletes its output file first and writes through the fixed temporary file pbeam_boot.bea# (compile.erl remove_file/2 and save_binary_1/2), so two compilations into one directory can collide.
mkdir -p "$output"
# Remove the lists of the last build before the build changes a file, so that a run with --no-build never uses an old list with the files of a build that did not complete.
rm -f "$output/erts.paths" "$output/pbeam.beams" "$output/pbeam.unloaded"
work=$(mktemp -d "$output/compile.XXXXXX")
trap 'rm -rf "$work"' EXIT
# The terra_counter example calls the Terra library and its Counter application. The repository has no Terra source, so compile them from the pinned Terra commit (scripts/terra.sh) first. The Elixir compiler runs the macros of the Terra sources. The Terra sources do not get --warnings-as-errors, because they are third-party code.
if [[ $name == terra_counter ]]; then
  terra=$(bash scripts/terra.sh)
  terra_list=$(find "$terra/lib" -name '*.ex' | LC_ALL=C sort)
  terra_sources=()
  while IFS= read -r source; do
    terra_sources+=("$source")
  done <<<"$terra_list"
  step "Compile the Terra sources of $terra"
  elixirc -o "$work" "${terra_sources[@]}" "$terra/examples/counter_app.exs"
fi
step "Compile ${sources[*]}"
elixirc --warnings-as-errors -o "$work" -pa "$work" "${sources[@]}"
[[ -f $work/$entry.beam ]] || fail "examples/$name defines no module ${entry#Elixir.}"
step "Compile examples/hello_world/pbeam_boot.erl"
erlc +deterministic -o "$work" examples/hello_world/pbeam_boot.erl
examples=()
for beam in "$work"/*.beam; do
  file=${beam##*/}
  mv -f "$beam" "$output/$file"
  [[ $file == pbeam_boot.beam ]] || examples+=("$output/$file")
done

# The boot modules that pbeam_boot starts, and the modules that these modules start with a module name in a variable, for example the child modules of a supervisor. The imports chunks do not give these modules.
boot_modules="erlang prim_tty proc_lib application application_controller lists os unicode gen_event gen prim_tty_sighandler erl_signal_handler standard_error supervisor_bridge user_sup user_drv group gen_statem io io_lib io_lib_format proplists queue gen_server logger logger_config edlin edlin_key logger_server logger_simple_h logger_sup supervisor logger_std_h logger_h_common logger_olp logger_proxy logger_handler_watcher logger_formatter maps init erts_internal"
# The pbeam binary loads only the BEAM files that it gets. Give it each OTP and Elixir module that the example, pbeam_boot, and the boot modules call, directly or through other modules (scripts/example_beams.exs). The script writes each called module that the pbeam run does not get, and the reason, into pbeam.unloaded.
# A command substitution, not a process substitution, so that set -e stops the run when the command fails.
step "Consolidate the Elixir protocols and find the OTP and Elixir modules that the example calls"
called_list=$(elixir scripts/example_beams.exs "$consolidated" "$work/pbeam.unloaded" "$boot_modules" "${examples[@]}" "$output/pbeam_boot.beam")
# The code path of the ERTS run. Each -pa option puts its directory before the directories of the previous -pa options (erl -pa A -pa B gives the code path B, A, and then OTP), so the last option puts the consolidated protocols before the unconsolidated protocols of Elixir.
paths=("$output")
called_beams=()
while read -r kind item; do
  case $kind in
    pbeam) called_beams+=("$item") ;;
    erts) paths+=("$item") ;;
  esac
done <<<"$called_list"
paths+=("$consolidated")
printf '%s\n' "${paths[@]}" >"$work/erts.paths"

# The BEAM files of the pbeam run.
beams=("${examples[@]}" ${called_beams[@]+"${called_beams[@]}"} "$output/pbeam_boot.beam")
printf '%s\n' "${beams[@]}" >"$work/pbeam.beams"

# Rename the lists last, each atomically. When a run with --no-build finds a list, each file of the list is complete.
mv -f "$work/pbeam.unloaded" "$output/pbeam.unloaded"
mv -f "$work/erts.paths" "$output/erts.paths"
mv -f "$work/pbeam.beams" "$output/pbeam.beams"
rm -rf "$work"
trap - EXIT

if [[ $erts == true ]]; then run "${paths[@]}"; fi
run "${beams[@]}"
