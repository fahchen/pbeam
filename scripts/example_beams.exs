# Consolidate the Elixir protocols for the given example BEAM files, and find the BEAM file of each OTP and Elixir module that the example and the boot modules call, directly or through other modules.
# Usage: elixir scripts/example_beams.exs <consolidated directory> <unloaded list> <boot modules> <example BEAM file>...
# The example BEAM files, with pbeam_boot.beam, are in one directory. The boot modules are one argument, with a space between two module names.
#
# `mix run` runs Mix.Tasks.Compile.Protocols, which consolidates each protocol of the code path that is not in OTP, with the implementations of the same code path (Mix.Compilers.Protocol, Elixir 1.20.4 lib/mix/lib/mix/compilers/protocol.ex:107-170). This script does the same step over the directory of the example and the Elixir code path, and writes each consolidated protocol to the consolidated directory. A consolidated protocol finds its implementation with a pattern match. An unconsolidated protocol calls Code.ensure_compiled/1, which calls code:ensure_loaded/1 (Elixir 1.20.4 lib/elixir/lib/protocol.ex:956-975).
# The script consolidates only the protocols of Elixir. A protocol of the example stays unconsolidated.
#
# A module is called when it is a boot module, or when it is in the imports chunk of an example BEAM file or of a called module (beam_lib:chunks/2). Each implementation of a called consolidated protocol is also called, because the protocol calls the implementation that it finds at run time. The pbeam binary loads only the BEAM files that it gets, so it needs this transitive closure, not only the modules that the example imports. The imports chunk does not have the modules that a run calls with a module name in a variable, for example the child modules of a supervisor. The boot modules give these modules for the boot.
# A called module is not given to the pbeam binary when the code path has no BEAM file for it, or when the PBeam loader does not load its BEAM file (@not_loaded). The script does not search the imports of such a module. It writes each one to the unloaded list, one line for each module: the module, the BEAM file (empty when there is none), and the reason, with a tab between them. A call to such a module stops the pbeam run with the runtime error `unresolved external MFA`.
#
# The script prints one line for each called module that is not an example module, sorted by the module name. A line `pbeam <file>` gives the BEAM file for the pbeam run. For a consolidated protocol, it is the file in the consolidated directory. A line `erts <directory>` follows the first BEAM file of each directory that is not in OTP and not the consolidated directory. The ERTS run needs these directories in its code path.

defmodule ExampleBeams do
  # Each module of the pinned OTP and Elixir that the PBeam loader does not load, with the load error. The loader stops at the first instruction that it cannot decode (src/beam/code/instruction.rs, skip_unsupported_instruction). The test gives_the_modules_that_the_example_calls_through_other_modules_as_pinned_erts of tests/feature/example.rs loads each file with the pbeam binary and compares the error. Remove a module when the loader learns its instruction.
  @not_loaded %{
    :calendar => "unsupported BEAM opcode 185 (bif3)",
    :crypto => "unsupported BEAM opcode 149 (on_load)",
    :elixir_errors => "unsupported BEAM opcode 132 (bs_match_string)",
    :eval_bits => "unsupported BEAM opcode 118 (bs_get_float2)",
    :inet_db => "unsupported BEAM opcode 185 (bif3)",
    :io_lib_fread => "unsupported BEAM opcode 185 (bif3)",
    :rand => "unsupported BEAM opcode 102 (fnegate)",
    :shell_docs => "unsupported BEAM opcode 132 (bs_match_string)",
    IEx.Evaluator => "unsupported BEAM opcode 132 (bs_match_string)"
  }

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
    if Map.has_key?(found, module) or Map.has_key?(unloaded, module) do
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

  # The BEAM file of a module, as code:which/1 gives it. A preloaded module of ERTS, such as erlang, has its file in the ebin directory of erts.
  defp beam_file(module) do
    case :code.which(module) do
      :preloaded -> :filename.join([:code.lib_dir(:erts), ~c"ebin", ~c"#{module}.beam"])
      path when is_list(path) -> path
      _ -> nil
    end
  end
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

lines =
  for {module, {path, reason}} <- Enum.sort(unloaded),
      do: [Atom.to_string(module), ?\t, path, ?\t, reason, ?\n]

File.write!(unloaded_list, lines)

consolidated_dir = String.to_charlist(output)

found
|> Map.drop(Map.keys(own))
|> Enum.sort()
|> Enum.reduce(MapSet.new(), fn {_module, path}, directories ->
  IO.puts("pbeam #{path}")
  directory = :filename.dirname(path)

  if directory == consolidated_dir or MapSet.member?(directories, directory) or
       :lists.prefix(otp, path) do
    directories
  else
    IO.puts("erts #{directory}")
    MapSet.put(directories, directory)
  end
end)
