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
  case :xmerl_xpath.string(~c"@#{name}", node) do
    [value] -> List.to_string(elem(value, 8))
    _ -> ""
  end
end

covered = coverage_attribute.("lines-covered")
valid = coverage_attribute.("lines-valid")
IO.puts("Application line coverage: #{covered}/#{valid}")

if valid == 0 or covered != valid do
  for class <- :xmerl_xpath.string(~c"//class", document),
      line <- :xmerl_xpath.string(~c"lines/line[@hits='0']", class) do
    IO.puts("UNCOVERED #{attribute.(class, "filename")}:#{attribute.(line, "number")}")
  end

  raise("application line coverage must be 100%")
end
