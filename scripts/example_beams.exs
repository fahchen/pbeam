# Print the path of the pinned BEAM file of each Elixir module that the given example BEAM files call, directly or through other Elixir modules, one path for each line.
# Usage: elixir scripts/example_beams.exs <example BEAM file>...
# A module is called when it is in the imports chunk of an example BEAM file or of a called Elixir module (beam_lib:chunks/2).
# The modules of the example are not printed. A module that a call finds only at run time, such as a protocol implementation, is not printed.

defmodule ExampleBeams do
  def called(path) do
    {:ok, {_module, [imports: imports]}} = :beam_lib.chunks(path, [:imports])

    for {module, _function, _arity} <- imports,
        String.starts_with?(Atom.to_string(module), "Elixir."),
        uniq: true,
        do: module
  end

  def closure([], found), do: found

  def closure([module | rest], found) do
    if Map.has_key?(found, module) do
      closure(rest, found)
    else
      path =
        case :code.which(module) do
          path when is_list(path) -> path
          _ -> raise "no BEAM file of #{inspect(module)} in the pinned Elixir"
        end

      closure(called(path) ++ rest, Map.put(found, module, path))
    end
  end
end

examples = System.argv()
own = Map.new(examples, &{&1 |> Path.basename(".beam") |> String.to_atom(), &1})

examples
|> Enum.flat_map(&ExampleBeams.called(String.to_charlist(&1)))
|> ExampleBeams.closure(own)
|> Map.drop(Map.keys(own))
|> Enum.sort()
|> Enum.each(fn {_module, path} -> IO.puts(path) end)
