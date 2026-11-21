Code.require_file("ci_task.exs", __DIR__)
Code.require_file("ci_frequency.exs", __DIR__)

defmodule PBeamTest do
  def run do
    # CI checks the Mix projects in a separate job. Its Rust slices prepare only the artifacts.
    run!("elixir", ["scripts/fixture_checks.exs"])

    partition = System.get_env("PBEAM_TEST_PARTITION", "")
    frequency_args = frequency_args()

    if frequency_args == [] do
      IO.puts("CI frequency: full test set")
    else
      IO.puts("CI frequency: reduced ERTS-only reference checks")
    end

    nextest = [
      "nextest",
      "run",
      "--locked",
      "--workspace",
      "--all-features",
      "--status-level",
      "slow",
      "--final-status-level",
      "slow",
      "--no-fail-fast"
    ]

    partition_args = if partition == "", do: [], else: ["--partition", partition]
    run!("cargo", nextest ++ frequency_args ++ partition_args)

    # One CI slice checks the inventories against the compiled list.
    if String.starts_with?(partition, "slice:1/"), do: verify_inventories()
  end

  defp frequency_args do
    if System.get_env("PBEAM_CI_EVENT") == "pull_request" do
      paths =
        CITask.changed_paths(
          System.fetch_env!("PBEAM_CI_BASE"),
          System.fetch_env!("PBEAM_CI_HEAD")
        )

      # Changes to tests, fixtures, and tool pins require the full list.
      if paths != [] and
           Enum.all?(paths, &(String.starts_with?(&1, "src/") and String.ends_with?(&1, ".rs"))) do
        ["-E", CIFrequency.filter(CIFrequency.identities())]
      else
        []
      end
    else
      []
    end
  end

  defp verify_inventories do
    suffix = 12 |> :crypto.strong_rand_bytes() |> Base.url_encode64(padding: false)
    dir = Path.join(System.tmp_dir!(), "pbeam-test-classes.#{suffix}")
    File.mkdir!(dir)

    try do
      metadata = Path.join(dir, "metadata.json")
      tests = Path.join(dir, "tests.json")

      list = [
        "nextest",
        "list",
        "--locked",
        "--workspace",
        "--all-features",
        "--profile",
        "ci",
        "--message-format",
        "json"
      ]

      cases = CIFrequency.identities()
      full = run!("cargo", list, into: "")
      reduced = run!("cargo", list ++ ["-E", CIFrequency.filter(cases)], into: "")
      CIFrequency.verify_lists(full, reduced, cases)

      File.write!(
        metadata,
        run!("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"], into: "")
      )

      File.write!(tests, full)

      for scope <- ~w(feature unit integration) do
        run!("elixir", ["scripts/test_classes.exs", "--strict", "--scope", scope, metadata, tests])
      end
    after
      File.rm_rf!(dir)
    end
  end

  defp run!(executable, args, opts \\ [into: IO.stream(:stdio, :line)]) do
    case System.cmd(executable, args, opts) do
      {output, 0} -> output
      {_, status} -> raise "#{Enum.join([executable | args], " ")} failed with status #{status}"
    end
  end
end

PBeamTest.run()
