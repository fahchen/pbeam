Code.require_file("ci_fixture_artifact.exs", __DIR__)
Code.require_file("test_selection.exs", __DIR__)

defmodule PBeamTest do
  def run do
    prebuilt? = System.get_env("PBEAM_PREBUILT_TESTS") == "1"
    build_args = build_args(prebuilt?)
    excluded = TestSelection.identities()
    selection_args = ["-E", TestSelection.filter(excluded)]

    if System.get_env("PBEAM_VERIFY_TEST_INVENTORIES") == "1" do
      verify_inventories(build_args, prebuilt?, selection_args, excluded)
    end

    nextest = [
      "nextest",
      "run",
      "--status-level",
      "slow",
      "--final-status-level",
      "slow",
      "--no-fail-fast"
    ]

    run!("cargo", nextest ++ build_args ++ selection_args)
  end

  defp build_args(false) do
    run!("elixir", ["scripts/fixture_checks.exs"])
    ~w(--locked --workspace --all-features)
  end

  defp build_args(true) do
    CIFixtureArtifact.verify!()

    for path <- ~w(target/nextest/cargo-metadata.json target/nextest/binaries-metadata.json) do
      unless File.regular?(path), do: raise("missing prebuilt test metadata: #{path}")
    end

    [
      "--workspace-remap",
      File.cwd!(),
      "--target-dir-remap",
      Path.expand("target"),
      "--cargo-metadata",
      "target/nextest/cargo-metadata.json",
      "--binaries-metadata",
      "target/nextest/binaries-metadata.json"
    ]
  end

  defp verify_inventories(build_args, prebuilt?, selection_args, excluded) do
    suffix = 12 |> :crypto.strong_rand_bytes() |> Base.url_encode64(padding: false)
    dir = Path.join(System.tmp_dir!(), "pbeam-test-classes.#{suffix}")
    File.mkdir!(dir)

    try do
      metadata = inventory_metadata(dir, prebuilt?)
      tests = Path.join(dir, "tests.json")

      list =
        [
          "nextest",
          "list",
          "--profile",
          "ci",
          "--message-format",
          "json"
        ] ++ build_args

      full = run!("cargo", list, into: "")
      selected = run!("cargo", list ++ selection_args, into: "")
      TestSelection.verify_lists(full, selected, excluded)

      File.write!(tests, full)

      for scope <- ~w(feature unit integration) do
        run!("elixir", ["scripts/test_classes.exs", "--strict", "--scope", scope, metadata, tests])
      end
    after
      File.rm_rf!(dir)
    end
  end

  defp inventory_metadata(dir, true) do
    metadata = "target/nextest/cargo-metadata.json" |> File.read!() |> JSON.decode!()
    members = metadata |> Map.fetch!("workspace_members") |> MapSet.new()

    # The inventory uses the same workspace packages as cargo metadata --no-deps.
    metadata =
      Map.update!(metadata, "packages", fn packages ->
        Enum.filter(packages, &MapSet.member?(members, Map.fetch!(&1, "id")))
      end)

    path = Path.join(dir, "metadata.json")
    File.write!(path, JSON.encode!(metadata))
    path
  end

  defp inventory_metadata(dir, false) do
    path = Path.join(dir, "metadata.json")
    File.write!(path, run!("cargo", ~w(metadata --locked --no-deps --format-version 1), into: ""))
    path
  end

  defp run!(executable, args, opts \\ [into: IO.stream(:stdio, :line)]) do
    case System.cmd(executable, args, opts) do
      {output, 0} -> output
      {_, status} -> raise "#{Enum.join([executable | args], " ")} failed with status #{status}"
    end
  end
end

PBeamTest.run()
