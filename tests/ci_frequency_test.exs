Code.require_file("../scripts/ci_frequency.exs", __DIR__)
ExUnit.start()

defmodule CIFrequencyTest do
  use ExUnit.Case, async: true
  import ExUnit.CaptureIO

  test "reviewed cases have ERTS-only facets" do
    rows = File.stream!(Path.join(__DIR__, "../scripts/ci_erts_only.txt"))
    assert MapSet.size(CIFrequency.identities()) == Enum.count(rows)
  end

  test "selection requires the exact difference" do
    retained = {"pbeam-runtime::a", "retained"}
    deferred = {"pbeam-runtime::a", "deferred"}
    other = {"pbeam-runtime::b", "other"}
    full = MapSet.new([retained, deferred])
    cases = MapSet.new([deferred])

    assert :ok = CIFrequency.verify(full, MapSet.new([retained]), cases)

    for reduced <- [full, MapSet.new(), MapSet.new([retained, other])] do
      assert_raise ArgumentError, ~r/unexpected selection/, fn ->
        CIFrequency.verify(full, reduced, cases)
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
             CIFrequency.verify_lists(list.("matches"), list.("mismatch"), cases)
           end) == "Frequency selection: 2 full, 1 PR, 1 ERTS-only\n"
  end
end
