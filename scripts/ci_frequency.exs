defmodule CIFrequency do
  def identities do
    rows = lines("ci_erts_only.txt")

    cases =
      MapSet.new(rows, fn row ->
        case String.split(row, "|") do
          [binary, name] ->
            if Enum.all?([binary, name], &Regex.match?(~r/\A[\w:-]+\z/u, &1)) do
              {binary, name}
            else
              raise ArgumentError, "invalid case: #{row}"
            end

          _ ->
            raise ArgumentError, "invalid case: #{row}"
        end
      end)

    if rows == [] or MapSet.size(cases) != length(rows) do
      raise ArgumentError, "the ERTS-only list is empty or has duplicates"
    end

    inventory =
      for path <- ["test_classes.exs", "test_integration_classes.txt"],
          line <- lines(path),
          [binary, name, purposes, dependencies] <- [String.split(String.trim(line), "|")],
          into: %{} do
        {{binary, name}, {String.split(purposes, ","), String.split(dependencies, ",")}}
      end

    Enum.each(cases, fn identity ->
      {purposes, dependencies} = Map.get(inventory, identity, {[], []})

      if "erts_only" not in purposes or "pbeam_process" in dependencies or
           "pbeam_tool" in dependencies do
        raise ArgumentError, "case is not reviewed as ERTS-only: #{inspect(identity)}"
      end
    end)

    cases
  end

  def filter(cases) do
    terms =
      cases
      |> Enum.sort()
      |> Enum.map_join(" | ", fn {binary, name} ->
        "(binary_id(=#{binary}) & test(=#{name}))"
      end)

    "not (#{terms})"
  end

  def verify(full, reduced, cases) do
    excluded = MapSet.difference(full, reduced)
    added = MapSet.difference(reduced, full)

    if excluded != cases or MapSet.size(added) != 0 do
      missing = excluded |> MapSet.difference(cases) |> Enum.sort()
      stale = cases |> MapSet.difference(excluded) |> Enum.sort()

      raise ArgumentError,
            "unexpected selection: missing=#{inspect(missing)}, stale=#{inspect(stale)}, added=#{inspect(Enum.sort(added))}"
    end

    :ok
  end

  def verify_lists(full_list, reduced_list, cases) do
    full = selected(full_list)
    reduced = selected(reduced_list)
    verify(full, reduced, cases)

    IO.puts(
      "Frequency selection: #{MapSet.size(full)} full, #{MapSet.size(reduced)} PR, #{MapSet.size(cases)} ERTS-only"
    )
  end

  def run(["filter"]), do: IO.puts(filter(identities()))

  def run(["verify", full, reduced]) do
    verify_lists(File.read!(full), File.read!(reduced), identities())
  end

  def run(_) do
    raise ArgumentError,
          "usage: elixir scripts/ci_frequency.exs filter | verify FULL_LIST REDUCED_LIST"
  end

  defp selected(list) do
    suites = list |> JSON.decode!() |> Map.fetch!("rust-suites")

    for {binary, suite} <- suites,
        {name, test} <- Map.fetch!(suite, "testcases"),
        test |> Map.fetch!("filter-match") |> Map.fetch!("status") == "matches",
        into: MapSet.new() do
      {binary, name}
    end
  end

  defp lines(path) do
    __DIR__
    |> Path.join(path)
    |> File.stream!()
    |> Enum.map(&String.trim_trailing(&1, "\n"))
  end
end

if Path.expand(List.to_string(List.first(:init.get_plain_arguments(), ~c""))) == __ENV__.file do
  CIFrequency.run(System.argv())
end
