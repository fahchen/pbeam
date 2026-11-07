# Check the inventory report with small nextest lists. These tests do not run
# the feature binaries or change the required CI test set.
ExUnit.start()

defmodule TestClassesReportTest do
  use ExUnit.Case, async: true

  setup do
    directory =
      Path.join(
        System.tmp_dir!(),
        "pbeam-test-classes-#{System.pid()}-#{System.unique_integer([:positive])}"
      )

    File.mkdir_p!(directory)
    on_exit(fn -> File.rm_rf!(directory) end)
    %{directory: directory}
  end

  defp report(directory, target, cases, arguments \\ []) do
    metadata = %{
      "packages" => [
        %{
          "name" => "pbeam-runtime",
          "targets" => [
            %{
              "name" => "stty",
              "kind" => ["test"],
              "src_path" => "/workspace/tests/feature/stty.rs"
            }
          ]
        }
      ]
    }

    suite = %{
      "testcases" =>
        Map.new(cases, fn {name, status} -> {name, %{"filter-match" => %{"status" => status}}} end)
    }

    list = %{"rust-suites" => if(target, do: %{"pbeam-runtime::stty" => suite}, else: %{})}
    metadata_path = Path.join(directory, "metadata.json")
    list_path = Path.join(directory, "list.json")
    File.write!(metadata_path, :json.encode(metadata))
    File.write!(list_path, :json.encode(list))

    System.cmd("elixir", ["scripts/test_classes.exs"] ++ arguments ++ [metadata_path, list_path],
      stderr_to_stdout: true
    )
  end

  test "reports selected known cases and their facets", %{directory: directory} do
    cases = [
      {"sets_the_terminal_mode_with_stty_through_a_port_as_pinned_erts", "matches"},
      {"reports_the_exit_status_of_stty_without_a_terminal_as_pinned_erts", "mismatch"}
    ]

    {output, status} = report(directory, true, cases, ["--cases", "--dependency", "pty"])
    assert status == 0
    assert output =~ "cases: 2; selected: 1"
    assert output =~ "Matching both requested facets: 1"
    assert output =~ "sets_the_terminal_mode_with_stty_through_a_port_as_pinned_erts"
    refute output =~ "reports_the_exit_status_of_stty_without_a_terminal_as_pinned_erts: purpose="
  end

  test "rejects a newly unrecorded case", %{directory: directory} do
    {output, status} = report(directory, true, [{"new_test", "matches"}])
    assert status != 0
    assert output =~ "unrecorded: pbeam-runtime::stty new_test"
  end

  test "rejects an omitted feature target", %{directory: directory} do
    {output, status} = report(directory, false, [])
    assert status != 0
    assert output =~ "nextest list omits feature targets"
  end

  test "strict mode rejects missing inventory entries on this host", %{directory: directory} do
    cases = [{"sets_the_terminal_mode_with_stty_through_a_port_as_pinned_erts", "matches"}]
    {output, status} = report(directory, true, cases, ["--strict"])
    assert status != 0
    assert output =~ "not listed on this platform"
  end
end
