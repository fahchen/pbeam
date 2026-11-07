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

  defp report(directory, target, cases, arguments \\ [], integration_target \\ "arith") do
    unit = Enum.chunk_every(arguments, 2, 1, :discard) |> Enum.any?(&(&1 == ["--scope", "unit"]))

    integration =
      Enum.chunk_every(arguments, 2, 1, :discard)
      |> Enum.any?(&(&1 == ["--scope", "integration"]))

    name =
      cond do
        unit -> "pbeam_runtime"
        integration -> integration_target
        true -> "stty"
      end

    kind = if unit, do: ["lib"], else: ["test"]

    source =
      cond do
        unit -> "/workspace/src/lib.rs"
        integration -> "/workspace/tests/integration/#{name}.rs"
        true -> "/workspace/tests/feature/stty.rs"
      end

    binary = if unit, do: "pbeam-runtime", else: "pbeam-runtime::" <> name

    metadata = %{
      "packages" => [
        %{
          "name" => "pbeam-runtime",
          "targets" => [
            %{
              "name" => name,
              "kind" => kind,
              "src_path" => source
            }
          ]
        }
      ]
    }

    suite = %{
      "testcases" =>
        Map.new(cases, fn {name, status} -> {name, %{"filter-match" => %{"status" => status}}} end)
    }

    list = %{"rust-suites" => if(target, do: %{binary => suite}, else: %{})}
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

  test "reports the macOS-only case as excluded on Linux", %{directory: directory} do
    cases = [{"sets_the_terminal_mode_with_stty_through_a_port_as_pinned_erts", "matches"}]
    {linux, _} = report(directory, true, cases, ["--platform", "linux"])
    {macos, _} = report(directory, true, cases, ["--platform", "macos"])

    name =
      "pbeam-runtime::example restores_the_terminal_before_the_result_of_terra_counter_as_interactive_erts"

    assert linux =~ "platform excluded: " <> name
    assert macos =~ "not listed on this platform: " <> name
  end

  test "reports library cases separately from feature cases", %{directory: directory} do
    cases = [{"beam::module::tests::reports_oversized_arity", "matches"}]
    {output, status} = report(directory, true, cases, ["--scope", "unit", "--cases"])
    assert status == 0
    assert output =~ "Unit targets: 1; cases: 1; selected: 1"

    assert output =~
             "beam::module::tests::reports_oversized_arity: purpose=pure_logic; dependencies=none"

    refute output =~ "unrecorded: pbeam-runtime "
  end

  test "rejects a new unrecorded library case", %{directory: directory} do
    {output, status} =
      report(directory, true, [{"new_unit_case", "matches"}], ["--scope", "unit"])

    assert status != 0
    assert output =~ "unrecorded: pbeam-runtime new_unit_case"
  end

  test "strict unit report rejects an unreviewed case", %{directory: directory} do
    cases = [
      {"runtime::mailbox::tests::caches_marker_references_when_priority_messages_arrive",
       "matches"}
    ]

    {output, status} = report(directory, true, cases, ["--scope", "unit", "--strict"])
    assert status != 0
    assert output =~ "Cases with unknown purpose or dependencies: 1; unrecorded: 0"
  end

  test "recognizes Linux-only library cases on macOS", %{directory: directory} do
    cases = [{"beam::module::tests::reports_oversized_arity", "matches"}]
    {output, _status} = report(directory, true, cases, ["--scope", "unit", "--platform", "macos"])
    {linux, _status} = report(directory, true, cases, ["--scope", "unit", "--platform", "linux"])

    name = "pbeam-runtime runtime::sockets::tests::counts_an_mptcp_socket_as_tcp"
    assert output =~ "platform excluded: " <> name
    assert linux =~ "not listed on this platform: " <> name
  end

  test "reports measurement and nextest selection as separate facets", %{directory: directory} do
    cases = [{"term::costs::prints_the_cost_of_a_binary_view", "mismatch"}]
    {output, status} = report(directory, true, cases, ["--scope", "unit", "--cases"])
    assert status == 0
    assert output =~ "purpose=measurement; dependencies=none; selected=false"
  end

  test "reports integration cases separately", %{directory: directory} do
    cases = [{"adds_and_subtracts_integers_as_pinned_erts", "matches"}]
    {output, status} = report(directory, true, cases, ["--scope", "integration", "--cases"])
    assert status == 0
    assert output =~ "Integration targets: 1; cases: 1; selected: 1"
    assert output =~ "purpose=runtime_integration,erts_comparison"
    assert output =~ "dependencies=erts_process,pipes,filesystem,network"
  end

  test "rejects missing integration targets and new cases", %{directory: directory} do
    {missing, missing_status} = report(directory, false, [], ["--scope", "integration"])
    assert missing_status != 0
    assert missing =~ "nextest list omits integration targets"

    {new, new_status} =
      report(directory, true, [{"new_case", "matches"}], ["--scope", "integration"])

    assert new_status != 0
    assert new =~ "unrecorded: pbeam-runtime::arith new_case"
  end

  test "recognizes integration platform exclusions", %{directory: directory} do
    cases = [{"adds_and_subtracts_integers_as_pinned_erts", "matches"}]
    {mac, _} = report(directory, true, cases, ["--scope", "integration", "--platform", "macos"])
    {linux, _} = report(directory, true, cases, ["--scope", "integration", "--platform", "linux"])
    name = "pbeam-runtime::net stops_the_run_at_a_network_namespace"
    assert mac =~ "platform excluded: " <> name
    assert linux =~ "not listed on this platform: " <> name
  end

  test "distinguishes live ERTS comparison from a recorded result", %{directory: directory} do
    cases = [
      {"runs_the_utf16_and_utf32_match_instructions_as_pinned_erts", "matches"},
      {"runs_bs_match_string_as_pinned_erts", "matches"}
    ]

    {output, status} =
      report(directory, true, cases, ["--scope", "integration", "--cases"], "binary_match")

    assert status == 0

    assert output =~
             "runs_the_utf16_and_utf32_match_instructions_as_pinned_erts: purpose=runtime_integration; dependencies=network"

    assert output =~
             "runs_bs_match_string_as_pinned_erts: purpose=runtime_integration,erts_comparison; dependencies=erts_process,pipes,filesystem,network"
  end
end
