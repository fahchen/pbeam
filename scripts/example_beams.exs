# Print the path of the pinned BEAM file of each Elixir module that the given example BEAM files call, one path for each line.
# Usage: elixir scripts/example_beams.exs <example BEAM file>...
# A module is called when it is in the imports chunk of an example BEAM file (beam_lib:chunks/2).
# The modules of the example are not printed. A module that only another Elixir module calls, or that a call finds at run time, such as a protocol implementation, is not printed.

examples = System.argv()
own = MapSet.new(examples, &(&1 |> Path.basename(".beam") |> String.to_atom()))

examples
|> Enum.flat_map(fn path ->
  {:ok, {_module, [imports: imports]}} = :beam_lib.chunks(String.to_charlist(path), [:imports])
  for {module, _function, _arity} <- imports, do: module
end)
|> Enum.uniq()
|> Enum.filter(&(String.starts_with?(Atom.to_string(&1), "Elixir.") and &1 not in own))
|> Enum.sort()
|> Enum.each(fn module ->
  case :code.which(module) do
    path when is_list(path) -> IO.puts(path)
    _ -> raise "no BEAM file of #{inspect(module)} in the pinned Elixir"
  end
end)
