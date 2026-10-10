# Consolidate the Elixir protocols for the given example BEAM files, and print the path of the BEAM file of each Elixir module that the example calls, directly or through other Elixir modules, one path for each line.
# Usage: elixir scripts/example_beams.exs <consolidated directory> <example BEAM file>...
# The example BEAM files are in one directory.
#
# `mix run` runs Mix.Tasks.Compile.Protocols, which consolidates each protocol of the code path that is not in OTP, with the implementations of the same code path (Mix.Compilers.Protocol, Elixir 1.20.4 lib/mix/lib/mix/compilers/protocol.ex:107-170). This script does the same step over the directory of the example and the Elixir code path, and writes each consolidated protocol to the consolidated directory. A consolidated protocol finds its implementation with a pattern match. An unconsolidated protocol calls Code.ensure_compiled/1, which calls code:ensure_loaded/1 (Elixir 1.20.4 lib/elixir/lib/protocol.ex:956-975).
# The script consolidates only the protocols of Elixir. A protocol of the example stays unconsolidated.
#
# A module is called when it is in the imports chunk of an example BEAM file or of a called Elixir module (beam_lib:chunks/2). Each implementation of a called consolidated protocol is also called, because the protocol calls the implementation that it finds at run time.
# The modules of the example are not printed. For a consolidated protocol, the path in the consolidated directory is printed.

defmodule ExampleBeams do
  def called(path) do
    {:ok, {_module, [imports: imports]}} = :beam_lib.chunks(path, [:imports])

    for {module, _function, _arity} <- imports,
        String.starts_with?(Atom.to_string(module), "Elixir."),
        uniq: true,
        do: module
  end

  # Consolidate each protocol of the Elixir paths with the implementations of all paths. Return a map of each protocol to its consolidated BEAM file and its implementation modules.
  def consolidate(output, example_path, elixir_paths) do
    File.mkdir_p!(output)
    paths = [example_path | elixir_paths]

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

  def closure([], found, _consolidated), do: found

  def closure([module | rest], found, consolidated) do
    if Map.has_key?(found, module) do
      closure(rest, found, consolidated)
    else
      {path, impls} =
        case {consolidated, :code.which(module)} do
          {%{^module => entry}, _} -> entry
          {_, path} when is_list(path) -> {path, []}
          _ -> raise "no BEAM file of #{inspect(module)} in the pinned Elixir"
        end

      closure(called(path) ++ impls ++ rest, Map.put(found, module, path), consolidated)
    end
  end
end

[output | examples] = System.argv()
own = Map.new(examples, &{&1 |> Path.basename(".beam") |> String.to_atom(), &1})
example_path = examples |> hd() |> Path.dirname() |> String.to_charlist()
# The Elixir paths are the paths of the code path that are not in OTP, as Mix.Compilers.Protocol.consolidation_paths/0 selects them. The current directory is the repository root, which has no BEAM file.
otp = :code.lib_dir()
elixir_paths = for path <- :code.get_path(), not :lists.prefix(otp, path), do: path
consolidated = ExampleBeams.consolidate(output, example_path, elixir_paths)

examples
|> Enum.flat_map(&ExampleBeams.called(String.to_charlist(&1)))
|> ExampleBeams.closure(own, consolidated)
|> Map.drop(Map.keys(own))
|> Enum.sort()
|> Enum.each(fn {_module, path} -> IO.puts(path) end)
