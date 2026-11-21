defmodule CiTestDurations do
  def run(source, destination) do
    if File.exists?(source) do
      # SAX avoids atom creation from XML element and attribute names.
      {:ok, {nil, cases}, trailing} =
        :xmerl_sax_parser.stream(File.read!(source), [
          :disallow_entities,
          event_fun: &event/3,
          event_state: {nil, []}
        ])

      if String.trim(trailing) != "", do: raise("unexpected data after the JUnit report")

      rows =
        cases
        |> Enum.reverse()
        |> Enum.map(&JSON.encode_to_iodata!/1)
        |> Enum.intersperse(",\n  ")

      contents = if rows == [], do: "[]\n", else: ["[\n  ", rows, "\n]\n"]
      File.write!(destination, contents)
      IO.puts("Saved durations for #{length(cases)} tests.")
    else
      IO.puts("No nextest timing report was produced.")
    end
  end

  defp event({:startElement, [], ~c"testcase", _, attributes}, _, {nil, cases}) do
    fields = for {[], [], name, value} <- attributes, into: %{}, do: {name, List.to_string(value)}
    {seconds, ""} = fields |> Map.fetch!(~c"time") |> String.trim() |> Float.parse()

    current = %{
      binary: Map.fetch!(fields, ~c"classname"),
      test: Map.fetch!(fields, ~c"name"),
      seconds: seconds,
      outcome: "passed"
    }

    {{current, 0}, cases}
  end

  defp event({:startElement, uri, name, _, _}, _, {{current, depth}, cases}) do
    outcome =
      case {depth, uri, name, current.outcome} do
        {0, [], ~c"failure", _} -> "failure"
        {0, [], ~c"error", outcome} when outcome != "failure" -> "error"
        {0, [], ~c"skipped", "passed"} -> "skipped"
        _ -> current.outcome
      end

    {{%{current | outcome: outcome}, depth + 1}, cases}
  end

  defp event({:endElement, _, _, _}, _, {{current, 0}, cases}), do: {nil, [current | cases]}

  defp event({:endElement, _, _, _}, _, {{current, depth}, cases}) do
    {{current, depth - 1}, cases}
  end

  defp event(_, _, state), do: state
end

[source, destination] = System.argv()
CiTestDurations.run(source, destination)
