Code.require_file("test_classes.exs", __DIR__)

defmodule TestSelection do
  @comparison ~w(erts_comparison erts_reference otp_compatibility)
  @independent ~w(application_end_to_end cli host_integration interoperability measurement native_extension pbeam_internal pure_logic test_helper)

  def identities do
    TestClasses.inventory()
    |> Map.merge(TestClasses.integration_inventory())
    |> Enum.filter(fn {_, %{purpose: purposes}} ->
      "erts_only" in purposes or
        (Enum.any?(purposes, &(&1 in @comparison)) and
           Enum.all?(purposes, &(&1 not in @independent)))
    end)
    |> MapSet.new(&elem(&1, 0))
  end

  def filter(cases) do
    terms =
      cases
      |> Enum.group_by(&elem(&1, 0), &elem(&1, 1))
      |> Enum.sort()
      |> Enum.map_join(" | ", fn {binary, names} ->
        names = names |> Enum.sort() |> Enum.map_join("|", &Regex.escape/1)
        "(binary_id(=#{binary}) & test(/^(?:#{names})$/))"
      end)

    "not (#{terms})"
  end

  def verify(full, reduced, cases) do
    cases = MapSet.intersection(full, cases)
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
      "Rust test selection: #{MapSet.size(full)} full, #{MapSet.size(reduced)} selected, #{MapSet.size(MapSet.intersection(full, cases))} custom OTP probes excluded"
    )
  end

  def run(["filter"]), do: IO.puts(filter(identities()))

  def run(["verify", full, reduced]) do
    verify_lists(File.read!(full), File.read!(reduced), identities())
  end

  def run(_) do
    raise ArgumentError,
          "usage: elixir scripts/test_selection.exs filter | verify FULL_LIST REDUCED_LIST"
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
end

if Path.expand(List.to_string(List.first(:init.get_plain_arguments(), ~c""))) == __ENV__.file do
  TestSelection.run(System.argv())
end
