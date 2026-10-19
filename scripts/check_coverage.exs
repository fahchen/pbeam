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

attribute = fn node, name ->
  case :xmerl_xpath.string(String.to_charlist("@#{name}"), node) do
    [value] -> value |> elem(8) |> List.to_string()
    _ -> nil
  end
end

covered = coverage_attribute.("lines-covered")
valid = coverage_attribute.("lines-valid")
IO.puts("Application line coverage: #{covered}/#{valid}")

if valid == 0 or covered != valid do
  # Name each file with a line that no test runs, so the failure is
  # actionable without the coverage report.
  for class <- :xmerl_xpath.string(~c"//class", document),
      {rate, _} = Float.parse(attribute.(class, "line-rate")),
      rate < 1.0 do
    IO.puts("Lines without a test: #{attribute.(class, "filename")} (#{rate})")
  end

  raise("application line coverage must be 100%")
end
