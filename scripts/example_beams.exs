# Consolidate the Elixir protocols for the given example BEAM files, print the start set of the pbeam run, and write the code path of the ERTS run.
# Usage: elixir scripts/example_beams.exs <consolidated directory> <unloaded list> <erts path list> <boot modules> <example BEAM file>...
# The example BEAM files, with pbeam_boot.beam, are in one directory. The boot modules are one argument, with a space between two module names.
#
# `mix run` runs Mix.Tasks.Compile.Protocols, which consolidates each protocol of the code path that is not in OTP, with the implementations of the same code path (Mix.Compilers.Protocol, Elixir 1.20.4 lib/mix/lib/mix/compilers/protocol.ex:107-170). This script does the same step over the directory of the example and the Elixir code path, and writes each consolidated protocol to the consolidated directory. A consolidated protocol finds its implementation with a pattern match. An unconsolidated protocol calls Code.ensure_compiled/1, which calls code:ensure_loaded/1 (Elixir 1.20.4 lib/elixir/lib/protocol.ex:956-975).
# The script consolidates only the protocols of Elixir. A protocol of the example stays unconsolidated.
#
# The pbeam run gets a start set and loads each other module on demand through the pinned error_handler and code_server, as an interactive-mode ERTS run does. The command line gives the example modules and pbeam_boot.beam to the run, and the start set adds each boot module, each preloaded module of ERTS that PBeam loads from a BEAM file (preloaded/0, lib/sasl/src/systools_make.erl:1582-1589), and each consolidated protocol. The script prints each start file as a line `pbeam <file>`.
# The script writes the code path of the ERTS run into <erts path list>, one directory for each line: each directory of a called module that is not in OTP, in the order of the modules, and the consolidated directory last, so that a consolidated protocol comes before the unconsolidated protocol of the same name. The pbeam run gets the same directories with `--pa`, so the load on demand finds each module.
#
# The script writes the unloaded list: each called module that the PBeam loader does not load, with its BEAM file and the reason, one line for each module, with a tab between the fields. A called module is a boot module, or a module in the imports chunk of an example BEAM file or of a called module (beam_lib:chunks/2), directly or through other modules. A module of PBeam BIFs, such as pbeam_cap, is not a called module: the pbeam binary runs its functions, and no BEAM file defines it. Each implementation of a called consolidated protocol is also called, because the protocol calls the implementation that it finds at run time. The script does not search the imports of a module that it does not load. A call to an unloaded module stops the pbeam run with the runtime error `unresolved external MFA`, unless the code path has the module: then error_handler loads it on demand. The test runs_the_called_modules_of_an_example_on_demand_as_pinned_erts of tests/feature/example.rs checks this list.

defmodule ExampleBeams do
  # Each module of the pinned OTP and Elixir that the PBeam loader does not load, with the load error. The loader stops at the first instruction that it cannot decode (src/beam/code/instruction.rs, skip_unsupported_instruction). A start module cannot have an -on_load function (BeamProgram::load in src/runtime/interpreter.rs), and a policy run gives the whole list as its start set, so the list keeps crypto. The test runs_the_called_modules_of_an_example_on_demand_as_pinned_erts of tests/feature/example.rs loads each file with the pbeam binary and compares the error. Remove a module when the loader learns its instruction.
  @not_loaded %{
    :crypto => "the start module crypto has an -on_load function, which PBeam runs only for a module that loads in the run (on_load)",
    :elixir_errors => "unsupported BEAM opcode 132 (bs_match_string)",
    :shell_docs => "unsupported BEAM opcode 132 (bs_match_string)",
    IEx.Evaluator => "unsupported BEAM opcode 132 (bs_match_string)"
  }

  # Each module of PBeam BIFs that ERTS does not have. No BEAM file defines it: the pbeam binary runs its functions. pbeam_boot calls pbeam_cap:serve/2 and pbeam_cap:enter_runtime/2.
  @pbeam_modules [:pbeam_cap]

  # The modules that ERTS preloads (preloaded/0, lib/sasl/src/systools_make.erl:1582-1589) and that the PBeam loader loads from a BEAM file. The other preloaded modules are PBeam native functions with no BEAM file (erlang, erts_internal, prim_file) or modules that PBeam does not run (erts_dirty_process_signal_handler, erts_literal_area_collector, erts_trace_cleaner, socket_registry, and the ESOCK modules). ERTS preloads init, erl_prim_loader, and erts_code_purger, so code:which/1 gives preloaded for them and the start set resolves their file in the ebin directory of erts.
  @preloaded [:init, :erl_prim_loader, :erts_code_purger]

  def called(path) do
    {:ok, {_module, [imports: imports]}} = :beam_lib.chunks(path, [:imports])
    for {module, _function, _arity} <- imports, uniq: true, do: module
  end

  # Consolidate each protocol of the Elixir paths with the implementations of all paths. Return a map of each protocol to its consolidated BEAM file and its implementation modules.
  def consolidate(output, example_files, elixir_paths) do
    File.mkdir_p!(output)
    paths = [example_files | elixir_paths]

    for protocol <- Protocol.extract_protocols(elixir_paths), into: %{} do
      impls = Protocol.extract_impls(protocol, paths)
      {:ok, binary} = Protocol.consolidate(protocol, impls)
      path = Path.join(output, "#{Atom.to_string(protocol)}.beam")

      # Write a file of this run, then rename it, so that a parallel run of the same example reads a complete file.
      partial = "#{path}.#{System.pid()}"
      File.write!(partial, binary)
      File.rename!(partial, path)
      {protocol, {String.to_charlist(path), Enum.map(impls, &Module.concat(protocol, &1))}}
    end
  end

  # Return a map of each called module to its BEAM file, and a map of each module that the pbeam run does not get to its BEAM file and the reason.
  def closure([], found, unloaded, _consolidated), do: {found, unloaded}

  def closure([module | rest], found, unloaded, consolidated) do
    if Map.has_key?(found, module) or Map.has_key?(unloaded, module) or module in @pbeam_modules do
      closure(rest, found, unloaded, consolidated)
    else
      case {consolidated, beam_file(module)} do
        {%{^module => {path, impls}}, _} ->
          closure(
            called(path) ++ impls ++ rest,
            Map.put(found, module, path),
            unloaded,
            consolidated
          )

        {_, nil} ->
          closure(
            rest,
            found,
            Map.put(unloaded, module, {~c"", "no BEAM file in the code path"}),
            consolidated
          )

        {_, path} ->
          case @not_loaded do
            %{^module => reason} ->
              closure(rest, found, Map.put(unloaded, module, {path, reason}), consolidated)

            _ ->
              closure(called(path) ++ rest, Map.put(found, module, path), unloaded, consolidated)
          end
      end
    end
  end

  # The BEAM file of a module, as code:which/1 gives it. ERTS preloads init and erl_prim_loader, so code:which/1 gives preloaded for them and the file is in the ebin directory of erts.
  defp beam_file(module) do
    case :code.which(module) do
      :preloaded -> preloaded_file(module)
      :non_existing -> nil
      path when is_list(path) -> path
      _ -> nil
    end
  end

  defp preloaded_file(module) do
    path = :filename.join([:code.lib_dir(:erts), ~c"ebin", ~c"#{module}.beam"])
    if File.exists?(path), do: path, else: nil
  end

  # Return the start set of the pbeam run: the modules that load before the run. The start set is each boot module, each preloaded module of ERTS that PBeam loads from a BEAM file, and each consolidated protocol that a called module refers to. The example modules and pbeam_boot.beam are already on the command line of the run, so they are not in this list. A module with no BEAM file gives `nil`.
  def start(boot, consolidated, found) do
    boot_modules = Enum.map(String.split(boot), &String.to_atom/1)

    protocols = for module <- Map.keys(consolidated), Map.has_key?(found, module), do: module

    (boot_modules ++ @preloaded ++ protocols)
    |> Enum.uniq()
    |> Enum.sort()
    |> Enum.map(fn module ->
      case Map.fetch(consolidated, module) do
        {:ok, {path, _impls}} -> {module, path}
        :error -> {module, beam_file(module)}
      end
    end)
  end

  # Return the code path directories of the ERTS run: each directory that is not in OTP and not the consolidated directory, in the order of the files, with the consolidated directory last. `files` holds each start file and each BEAM file of a called module.
  def erts_paths(files, otp, consolidated_dir) do
    directories =
      for path <- files,
          directory = :filename.dirname(path),
          directory != consolidated_dir,
          not :lists.prefix(otp, path),
          do: directory

    Enum.uniq(directories) ++ [consolidated_dir]
  end

  # Return the lines of the build output: one line `pbeam <file>` for each start module, one line `called <file>` for each other called module, and then one line `erts <directory>` for each code path directory of the ERTS run: the directory of the example BEAM files first, then each directory of a called module. The caller reads the lines in order.
  def lines(start, found, own, example_dir, otp, consolidated_dir) do
    start_files = for {_module, path} <- start, path != nil, do: path

    called_files =
      for {module, path} <- found, not Map.has_key?(own, module), not start_member(start, module) do
        path
      end

    start_lines = for path <- start_files, do: "pbeam #{path}"
    called_lines = for path <- called_files, do: "called #{path}"

    erts_lines =
      for directory <-
            [example_dir | erts_paths(start_files ++ called_files, otp, consolidated_dir)],
          do: "erts #{directory}"

    start_lines ++ called_lines ++ erts_lines
  end

  defp start_member(start, module), do: Enum.any?(start, fn {name, _path} -> name == module end)
end

[output, unloaded_list, boot | examples] = System.argv()

own =
  Map.new(examples, &{&1 |> Path.basename(".beam") |> String.to_atom(), String.to_charlist(&1)})

# The example directory keeps the BEAM files of earlier builds, so give Protocol.extract_impls/2 the files of this build only, as a directory with its list of files.
example_dir = examples |> hd() |> Path.dirname() |> String.to_charlist()

example_files =
  {example_dir, Enum.map(examples, &(&1 |> Path.basename() |> String.to_charlist()))}

# The Elixir paths are the paths of the code path that are not in OTP, as Mix.Compilers.Protocol.consolidation_paths/0 selects them. The current directory is the repository root, which has no BEAM file.
otp = :code.lib_dir()
elixir_paths = for path <- :code.get_path(), not :lists.prefix(otp, path), do: path
consolidated = ExampleBeams.consolidate(output, example_files, elixir_paths)

roots =
  Enum.flat_map(Map.values(own), &ExampleBeams.called/1) ++
    Enum.map(String.split(boot), &String.to_atom/1)

{found, unloaded} = ExampleBeams.closure(roots, own, %{}, consolidated)

# The start set holds each consolidated protocol that a called module refers to, so it needs the closure.
start = ExampleBeams.start(boot, consolidated, found)

unloadable =
  for {module, nil} <- start do
    [Atom.to_string(module), ?\t, ~c"", ?\t, "no BEAM file in the code path", ?\n]
  end

unloaded_lines =
  for {module, {path, reason}} <- Enum.sort(unloaded),
      do: [Atom.to_string(module), ?\t, path, ?\t, reason, ?\n]

File.write!(unloaded_list, unloaded_lines ++ unloadable)

for line <- ExampleBeams.lines(start, found, own, example_dir, otp, String.to_charlist(output)) do
  IO.puts(line)
end
