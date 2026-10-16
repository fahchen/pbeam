#!/usr/bin/env bash
# Build and run the example examples/<name> on the pbeam binary, or on the pinned ERTS with --erts.
# The ERTS run uses one scheduler (+S 1), because the pbeam binary runs all processes on one scheduler thread. With more schedulers, ERTS can run two busy processes at the same time, and the order of their output changes from run to run.
# Usage: bash scripts/example.sh <name> [--erts] [--no-build] [--policy <module>]
# The example is the Elixir sources examples/<name>/*.ex. The entry is main/0 of the module with the camel-case name of the example, for example HelloWorld for hello_world.
# The script has two steps, the build and the run. The build writes all the files that a run needs into target/examples/<name>/. The run starts only erl or the pbeam binary. With --no-build, the script does not build: it reads a list of the last build and runs it. Thus the tests build each example once, and the ERTS run and the pbeam run execute the same compiled modules.
# The build compiles the example modules and pbeam_boot.beam into target/examples/<name>/, and consolidates the Elixir protocols into target/examples/<name>/consolidated/, as `mix run` consolidates them (scripts/example_beams.exs). It then writes the code path of the ERTS run into target/examples/<name>/erts.paths, one directory for each line. It writes the start set of the pbeam run into target/examples/<name>/pbeam.start, one BEAM file for each line: the boot modules, the example modules, pbeam_boot.beam, the preloaded modules of ERTS, and the consolidated protocols. The pbeam run loads each other module on demand through error_handler and code_server, as an interactive-mode ERTS run does. A policy run gives the whole list of the build (target/examples/<name>/pbeam.beams) instead, because the boot gate denies a message of a runtime process to a boot process. The build writes each called module that the PBeam loader does not load, with the reason, into target/examples/<name>/pbeam.unloaded.
# The terra_counter example also compiles the Terra sources of the pinned commit, which scripts/terra.sh downloads or finds in PBEAM_TERRA_SRC.
# With --policy <module>, the pbeam run gives --boot --policy <module> to the pbeam binary. The boot of pbeam_boot then runs with the boot authority, and the policy module of the build gives the capabilities of the entry when pbeam_boot enters the runtime (docs/capability-security-migration.md, section "Decisions of #463"). The ERTS run has no policy, so the script rejects --policy with --erts.
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
      policy=(--boot --policy "$2")
      shift
      ;;
    *) fail "unknown option $1" ;;
  esac
  shift
done
[[ $erts == false || ${#policy[@]} -eq 0 ]] || fail 'the ERTS run has no policy: give --policy without --erts'
[[ $name =~ ^[a-z][a-z0-9_]*$ ]] || fail "the name $name is not a lowercase name such as hello_world"
# The boot modules that pbeam_boot starts, and the modules that these modules start with a module name in a variable, for example the child modules of a supervisor. The imports chunks do not give these modules. The start set of the pbeam run is these modules, the example modules, pbeam_boot.beam, the preloaded modules of ERTS, and the consolidated protocols (scripts/example_beams.exs). The pbeam run gives the code path of the ERTS run with --pa and the root directory of the pinned OTP with --root, so error_handler and code_server load each other module on demand.
boot_modules="erlang prim_tty proc_lib application application_controller lists os unicode gen_event gen prim_tty_sighandler erl_signal_handler standard_error file_server supervisor_bridge user_sup user_drv group gen_statem io io_lib io_lib_format proplists queue gen_server logger logger_config edlin edlin_key logger_server logger_simple_h logger_sup supervisor logger_std_h logger_h_common logger_olp logger_proxy logger_handler_watcher logger_formatter maps init erts_internal erl_prim_loader prim_file error_handler"
sources=(examples/"$name"/*.ex)
[[ -e ${sources[0]} || -f examples/$name/mix.exs ]] || fail "examples/$name has no .ex file and no mix.exs"

entry=Elixir.
IFS=_ read -ra words <<<"$name"
for word in "${words[@]}"; do
  entry+=$(tr '[:lower:]' '[:upper:]' <<<"${word:0:1}")${word:1}
done
output=target/examples/$name
consolidated=$output/consolidated

# Run the example on the pinned ERTS with the code path directories, or on the pbeam binary with the code path directories, the start set of BEAM files, and the root directory. The code path directories are in the global list `paths`. The function does not return.
run() {
  if [[ $erts == true ]]; then
    local options=()
    for path in "${paths[@]}"; do
      options+=(-pa "$path")
    done
    step "Run ${entry#Elixir.}.main/0 on the pinned ERTS (erl +S 1)"
    exec erl +S 1 -noshell "${options[@]}" -eval "'$entry':main(), halt()."
  fi
  # The pbeam run gives the root directory of the pinned OTP, which erl gives to init with -root, so that pbeam_boot starts code_server (code.erl:1686), and the code path directories as --pa, so that error_handler and code_server load each other module on demand.
  local root
  root=$(erl -noshell -eval 'io:put_chars(code:root_dir()), halt().')
  local options=()
  for path in "${paths[@]}"; do
    options+=(--pa "$path")
  done
  if [[ -n ${PBEAM_BIN:-} ]]; then
    step "Run ${entry#Elixir.}.main/0 on $PBEAM_BIN with $# BEAM files"
    exec "$PBEAM_BIN" ${policy[@]+"${policy[@]}"} pbeam_boot start "$entry" main --root "$root" ${options[@]+"${options[@]}"} -- "$@"
  fi
  step "Build pbeam and run ${entry#Elixir.}.main/0 with $# BEAM files"
  exec cargo run --quiet --locked --bin pbeam -- ${policy[@]+"${policy[@]}"} pbeam_boot start "$entry" main --root "$root" ${options[@]+"${options[@]}"} -- "$@"
}

# A release example is a Mix project, examples/<name>/mix.exs, with a release that the build writes into target/examples/<name>/. The entry is main/0 of the Erlang module <name>_run of examples/<name>/<name>_run.erl. The build runs `mix release` and compiles the entry and pbeam_boot.erl into target/examples/<name>/pbeam/. The code path of the run is that directory, each lib/*/ebin directory of the release, and releases/<version>/consolidated last, so that the consolidated protocols come first. The start set of the pbeam run is the entry, pbeam_boot.beam, and the files of the boot modules of the pinned OTP. Both runs get RELEASE_ROOT and RELEASE_SYS_CONFIG as bin/<name> sets them. The run does not read the boot script of the release (docs/erts-differences.md, row "Boot").
if [[ -f examples/$name/mix.exs ]]; then
  [[ ${#policy[@]} -eq 0 ]] || fail 'a release example has no policy: give no --policy'
  release=target/examples/$name
  output=$release/pbeam
  entry=${name}_run
  [[ -f examples/$name/$entry.erl ]] || fail "examples/$name has no $entry.erl"
  if [[ $build == true ]]; then
    # The output of mix goes to stderr, so that stdout has only the output of the run.
    step "Build the release of examples/$name"
    (cd "examples/$name" && mix deps.get --check-locked && MIX_ENV=prod mix release --overwrite) >&2
    mkdir -p "$output"
    step "Compile examples/$name/$entry.erl and examples/hello_world/pbeam_boot.erl"
    erlc +deterministic -o "$output" "examples/$name/$entry.erl" examples/hello_world/pbeam_boot.erl
  fi
  [[ -f $output/$entry.beam && -f $release/releases/start_erl.data ]] || fail "$release has no build. Build the example first: run the script without --no-build"
  version=$(cut -d' ' -f2 "$release/releases/start_erl.data")
  paths=("$output" "$release"/lib/*/ebin "$release/releases/$version/consolidated")
  RELEASE_ROOT=$(cd "$release" && pwd -P)
  RELEASE_SYS_CONFIG=$RELEASE_ROOT/releases/$version/sys
  export RELEASE_ROOT RELEASE_SYS_CONFIG
  [[ $erts == false ]] || run
  # code:which/1 gives preloaded for a preloaded module, such as init, so its file is in the ebin directory of erts.
  # pbeam_boot spawns erts_code_purger before init, as erl_init.c does, and the other examples get it from @preloaded of scripts/example_beams.exs.
  read -ra boot_list <<<"$boot_modules erts_code_purger"
  start_list=$(erl -noshell -eval '[io:put_chars([F, $\n]) || A <- init:get_plain_arguments(), F <- [case code:which(list_to_atom(A)) of preloaded -> filename:join([code:lib_dir(erts), "ebin", A ++ ".beam"]); P -> P end], filelib:is_regular(F)], halt().' -extra "${boot_list[@]}")
  start=("$output/$entry.beam" "$output/pbeam_boot.beam")
  while IFS= read -r item; do
    start+=("$item")
  done <<<"$start_list"
  run "${start[@]}"
fi

# With --no-build, run the last build with the lists that it wrote: the start set of BEAM files, and the code path directories. A policy run loads each other module of the run before the entry: the boot gate denies a message of a runtime process to a boot process (docs/capability-security-migration.md), so a policy run gives the whole list of the build.
if [[ $build == false ]]; then
  list=$output/pbeam.start
  path_list=$output/erts.paths
  if [[ ${#policy[@]} -gt 0 ]]; then
    list=$output/pbeam.beams
    [[ -f $list ]] || fail "$list does not exist. Build the example first: run the script without --no-build"
  fi
  for required in "$list" "$path_list"; do
    [[ -f $required ]] || fail "$required does not exist. Build the example first: run the script without --no-build"
  done
  beams=()
  while IFS= read -r item; do
    beams+=("$item")
  done <"$list"
  paths=()
  while IFS= read -r item; do
    paths+=("$item")
  done <"$path_list"
  run "${beams[@]}"
fi

# The build tools elixirc and elixir start the elixir application, which writes a warning to stderr when the file name encoding is latin1. ERTS on Linux selects latin1 in the C locale. The example files have ASCII names, so the build uses UTF-8 file names in each locale. The erl runs of the example are not changed.
export ELIXIR_ERL_OPTIONS="${ELIXIR_ERL_OPTIONS:-} +fnu"

# Compile into a directory of this run, then rename each file into the output directory. A rename replaces a file atomically, so a run or a test that reads a file at the same time reads a complete file. erlc deletes its output file first and writes through the fixed temporary file pbeam_boot.bea# (compile.erl remove_file/2 and save_binary_1/2), so two compilations into one directory can collide.
mkdir -p "$output"
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
# Remove the lists of the last build before the first rename of a file into the output directory, so that a run with --no-build never uses an old list with the files of a build that did not complete. A build that stops before this point, for example a build without a Terra source, keeps the lists and the files of the last complete build.
rm -f "$output/erts.paths" "$output/pbeam.beams" "$output/pbeam.start" "$output/pbeam.unloaded"
examples=()
for beam in "$work"/*.beam; do
  file=${beam##*/}
  mv -f "$beam" "$output/$file"
  [[ $file == pbeam_boot.beam ]] || examples+=("$output/$file")
done

# A command substitution, not a process substitution, so that set -e stops the run when the command fails. The script prints one line for each start module (`pbeam <file>`), one line for each other called module (`called <file>`), and then one line for each code path directory of the ERTS run (`erts <directory>`).
step "Consolidate the Elixir protocols and find the start set of the pbeam run"
called_list=$(elixir scripts/example_beams.exs "$consolidated" "$work/pbeam.unloaded" "$boot_modules" "${examples[@]}" "$output/pbeam_boot.beam")
# The code path of the ERTS run. Each -pa option puts its directory before the directories of the previous -pa options (erl -pa A -pa B gives the code path B, A, and then OTP), so the last option puts the consolidated protocols before the unconsolidated protocols of Elixir.
start_beams=()
called_beams=()
path_directories=()
while read -r kind item; do
  case $kind in
    pbeam) start_beams+=("$item") ;;
    called) called_beams+=("$item") ;;
    erts) path_directories+=("$item") ;;
  esac
done <<<"$called_list"

# The start set of the pbeam run. Each other module of the build loads on demand from the code path (docs/erts-differences.md, row "Code loading").
start=("${examples[@]}" ${start_beams[@]+"${start_beams[@]}"} "$output/pbeam_boot.beam")
printf '%s\n' "${start[@]}" >"$work/pbeam.start"
# Each called module that the PBeam loader loads, before the run. A policy run gives this whole list, because the boot gate denies a message of a runtime process to a boot process.
beams=("${start[@]}" ${called_beams[@]+"${called_beams[@]}"})
printf '%s\n' "${beams[@]}" >"$work/pbeam.beams"
printf '%s\n' "${path_directories[@]}" >"$work/erts.paths"

# Rename the lists last, each atomically. When a run with --no-build finds a list, each file of the list is complete.
mv -f "$work/pbeam.unloaded" "$output/pbeam.unloaded"
mv -f "$work/erts.paths" "$output/erts.paths"
mv -f "$work/pbeam.beams" "$output/pbeam.beams"
mv -f "$work/pbeam.start" "$output/pbeam.start"
rm -rf "$work"
trap - EXIT

paths=("${path_directories[@]}")
# A policy run gives the whole list: the runtime-to-boot gate denies the load call of a runtime process to `code_server`.
if [[ ${#policy[@]} -gt 0 ]]; then run "${beams[@]}"; fi
run "${start[@]}"
