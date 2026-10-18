defmodule Mix.Tasks.Pbeam.Bundle do
  @moduledoc """
  Writes the PBeam application bundle of the release.

  The task builds the `pbeam_bundle` tool, resolves the start set with
  `scripts/example.sh --no-build --write-start`, and writes one bundle. Run
  `mix release` first; the task reads the release that the project config
  names.

      mix pbeam.bundle
      mix pbeam.bundle --output target/app.pbeamb --start-list target/start.list --tool target/release/pbeam_bundle
  """

  use Mix.Task

  @shortdoc "Writes the PBeam application bundle of the release"

  @impl Mix.Task
  def run(arguments) do
    {options, _, invalid} =
      OptionParser.parse(arguments,
        strict: [output: :string, start_list: :string, tool: :string]
      )

    if invalid != [] do
      Mix.raise("pbeam.bundle: unknown arguments #{inspect(invalid)}")
    end

    root = Path.expand("../..", File.cwd!())
    app = Mix.Project.config()[:app]
    release = release_directory(app)
    output = Keyword.get(options, :output) || Path.join(Path.dirname(release), "#{app}.pbeamb")
    start = Keyword.get(options, :start_list) || Path.join(release, "pbeam.start.list")

    tool =
      case Keyword.get(options, :tool) do
        nil ->
          run!("cargo", ["build", "--locked", "--release", "--bin", "pbeam_bundle"], root)
          Path.join(root, "target/release/pbeam_bundle")

        tool ->
          tool
      end

    run!(
      "bash",
      ["scripts/example.sh", Atom.to_string(app), "--no-build", "--write-start", start],
      root
    )

    run!(tool, ["bundle", release, output, "#{app}_run", "main", start], root)
    Mix.shell().info("Wrote #{output}")
  end

  defp release_directory(app) do
    path = Mix.Project.config()[:releases][app][:path]

    if path == nil do
      Mix.raise("pbeam.bundle: the project config names no directory for the release #{app}")
    end

    Path.expand(path)
  end

  defp run!(command, arguments, root) do
    {output, status} = System.cmd(command, arguments, cd: root, stderr_to_stdout: true)

    if status != 0 do
      Mix.raise("pbeam.bundle: #{command} failed with status #{status}:\n#{output}")
    end
  end
end
