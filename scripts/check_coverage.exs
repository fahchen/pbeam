# Read one or more Cobertura reports and require 100% application line coverage.
# CI runs the Rust tests in partitions, and each partition writes its own report from the same build. The reports have the same lines, so a line has a test when it has hits in one report or more.
paths = System.argv()
if paths == [], do: raise("coverage XML path is required")

attribute = fn node, name ->
  case :xmerl_xpath.string(String.to_charlist("@#{name}"), node) do
    [value] -> value |> elem(8) |> List.to_string()
    _ -> nil
  end
end

# The hits of each line, keyed by the file name and the line number.
lines =
  Enum.reduce(paths, %{}, fn path, lines ->
    contents = File.read!(path)
    contents = Regex.replace(~r/<!DOCTYPE [^>]+>\s*/, contents, "")
    {document, _} = :xmerl_scan.string(String.to_charlist(contents))

    for class <- :xmerl_xpath.string(~c"//class", document),
        filename = attribute.(class, "filename"),
        line <- :xmerl_xpath.string(~c"lines/line", class),
        reduce: lines do
      lines ->
        key = {filename, String.to_integer(attribute.(line, "number"))}
        hits = String.to_integer(attribute.(line, "hits"))
        Map.update(lines, key, hits, &max(&1, hits))
    end
  end)

uncovered = for {key, 0} <- lines, do: key
valid = map_size(lines)
IO.puts("Application line coverage: #{valid - length(uncovered)}/#{valid}")

if valid == 0 or uncovered != [] do
  # Name each file with a line that no test runs, so the failure is
  # actionable without the coverage report.
  for {filename, keys} <- Enum.group_by(uncovered, &elem(&1, 0)) |> Enum.sort() do
    numbers = keys |> Enum.map(&elem(&1, 1)) |> Enum.sort()
    IO.puts("Lines without a test: #{filename}: #{Enum.join(numbers, ", ")}")
  end

  raise("application line coverage must be 100%")
end
