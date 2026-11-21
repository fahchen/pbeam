Code.require_file("../scripts/test_selection.exs", __DIR__)
ExUnit.start()

defmodule TestSelectionTest do
  use ExUnit.Case, async: true
  import ExUnit.CaptureIO

  test "the inventories exclude custom OTP probes and keep independent coverage" do
    cases = TestSelection.identities()

    for identity <- [
          {"pbeam-runtime::alias", "pinned_erts_orders_priority_down_before_queued_messages"},
          {"pbeam-runtime::signal",
           "keeps_erts_alive_after_sighup_in_the_ignore_and_handle_modes"},
          {"pbeam-runtime::arith", "adds_and_subtracts_integers_as_pinned_erts"},
          {"pbeam-runtime::code_server",
           "calls_the_funs_of_identical_reloaded_code_as_pinned_erts"},
          {"pbeam-runtime::external_term", "decodes_each_term_type_as_pinned_erts"}
        ] do
      assert MapSet.member?(cases, identity)
    end

    for identity <- [
          {"pbeam-runtime::code_server",
           "gives_the_crypto_algorithm_lists_and_fips_status_as_pinned_erts"},
          {"pbeam-runtime::distribution", "connects_over_tls_in_both_directions_as_pinned_erts"},
          {"pbeam-runtime::stty",
           "sets_the_terminal_mode_with_stty_through_a_port_as_pinned_erts"},
          {"pbeam-runtime::release_boot",
           "boots_a_release_with_the_arguments_of_erl_as_pinned_erts"},
          {"pbeam-runtime::boot",
           "boots_the_standard_io_servers_before_the_entry_of_the_pbeam_binary"},
          {"pbeam-runtime::multischeduler", "loads_code_on_one_scheduler_and_runs_it_on_another"},
          {"pbeam-runtime::boot", "isolated_init_stop"},
          {"pbeam-runtime", "term::tests::gives_a_reference_its_erts_reference_numbers"}
        ] do
      refute MapSet.member?(cases, identity)
    end
  end

  test "selection requires the exact difference" do
    retained = {"pbeam-runtime::a", "retained"}
    deferred = {"pbeam-runtime::a", "deferred"}
    other = {"pbeam-runtime::b", "other"}
    full = MapSet.new([retained, deferred])
    unavailable = {"pbeam-runtime::platform_specific", "absent"}
    cases = MapSet.new([deferred, unavailable])

    assert :ok = TestSelection.verify(full, MapSet.new([retained]), cases)

    for reduced <- [full, MapSet.new(), MapSet.new([retained, other])] do
      assert_raise ArgumentError, ~r/unexpected selection/, fn ->
        TestSelection.verify(full, reduced, cases)
      end
    end
  end

  test "nextest lists distinguish excluded cases from matches" do
    binary = "pbeam-runtime::reference"

    list = fn status ->
      JSON.encode!(%{
        "rust-suites" => %{
          binary => %{
            "testcases" => %{
              "deferred" => %{"filter-match" => %{"status" => status}},
              "retained" => %{"filter-match" => %{"status" => "matches"}}
            }
          }
        }
      })
    end

    cases = MapSet.new([{binary, "deferred"}])

    assert capture_io(fn ->
             TestSelection.verify_lists(list.("matches"), list.("mismatch"), cases)
           end) == "Rust test selection: 2 full, 1 selected, 1 custom OTP probes excluded\n"
  end
end
