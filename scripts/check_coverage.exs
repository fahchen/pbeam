path = List.first(System.argv()) || raise "coverage XML path is required"
contents = File.read!(path)
contents = Regex.replace(~r/<!DOCTYPE [^>]+>\s*/, contents, "")
{document, _} = :xmerl_scan.string(String.to_charlist(contents))

coverage_attribute = fn name ->
  case :xmerl_xpath.string(~c"/coverage/@#{name}", document) do
    [attribute] -> attribute |> elem(8) |> List.to_string() |> String.to_integer()
    _ -> raise "coverage XML is missing #{name}"
  end
end

covered = coverage_attribute.("lines-covered")
valid = coverage_attribute.("lines-valid")
IO.puts("Application line coverage: #{covered}/#{valid}")

# Temporary debug: print each class-level line whose hits are zero.
for [_, filename, body] <- Regex.scan(~r/<class [^>]*filename="([^"]+)"[^>]*>(.*?)<\/class>/s, contents) do
  lines =
    case Regex.scan(~r/<lines>(.*?)<\/lines>/s, body) do
      [] -> ""
      matches -> matches |> List.last() |> Enum.at(1)
    end

  uncovered =
    Regex.scan(~r/<line ([^>]*)\/>/, lines)
    |> Enum.filter(fn [_, attributes] -> String.contains?(attributes, ~s(hits="0")) end)
    |> Enum.map(fn [_, attributes] ->
      Regex.run(~r/number="(\d+)"/, attributes) |> List.last()
    end)

  if uncovered != [] do
    IO.puts("UNCOVERED #{filename}: #{Enum.join(uncovered, ",")}")
  end
end

if valid == 0 or covered != valid, do: raise("application line coverage must be 100%")
