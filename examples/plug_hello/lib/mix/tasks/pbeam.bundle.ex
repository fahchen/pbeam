defmodule Mix.Tasks.Pbeam.Bundle do
  @moduledoc """
  Writes the PBeam application bundle of the release.

  The task builds the `pbeam_bundle` tool, resolves the start set (the
  preloaded modules) with `scripts/example.sh --no-build --write-start`, and
  writes one bundle. The entry `elixir:start_cli` is the `-s` call of
  `bin/<app> start`, which the launcher gives to the boot of the release. The
  NIF manifest gate of the tool reports each NIF function that PBeam does not
  bind and stops the task on a module that PBeam has no NIF library for. Run
  `mix release` first; the task reads the release that the project config
  names, or the directory of `--release`. Relative paths are relative to the
  directory of the command line.

      mix pbeam.bundle
      mix pbeam.bundle --output target/app.pbeamb --start-list target/start.list --tool target/release/pbeam_bundle
  """

  use Mix.Task

  @shortdoc "Writes the PBeam application bundle of the release"

  @impl Mix.Task
  def run(arguments) do
    {options, positional, invalid} =
      OptionParser.parse(arguments,
        strict: [output: :string, start_list: :string, tool: :string, release: :string]
      )

    if invalid != [] or positional != [] do
      Mix.raise("pbeam.bundle: unknown arguments #{inspect(invalid ++ positional)}")
    end

    cwd = File.cwd!()
    root = Path.expand("../..", cwd)
    app = Mix.Project.config()[:app]
    release = release_directory(app, options, cwd)
    output = Path.expand(Keyword.get(options, :output) || default_output(release, app), cwd)
    File.mkdir_p!(Path.dirname(output))

    start =
      Path.expand(
        Keyword.get(options, :start_list) || Path.join(release, "pbeam.start.list"),
        cwd
      )

    tool =
      case Keyword.get(options, :tool) do
        nil ->
          build_tool!(root)

        tool ->
          Path.expand(tool, cwd)
      end

    run!(
      "bash",
      [
        "scripts/example.sh",
        Atom.to_string(app),
        "--no-build",
        "--write-start",
        start,
        "--release",
        release
      ],
      root
    )

    # The NIF manifest gate of the tool prints each NIF function that PBeam
    # does not bind.
    gate = run!(tool, ["bundle", release, output, "elixir", "start_cli", start], root)
    Mix.shell().info(gate)
    Mix.shell().info("Wrote #{output}")
  end

  defp release_directory(app, options, cwd) do
    path =
      Keyword.get(options, :release) || Mix.Project.config()[:releases][app][:path]

    if path == nil do
      Mix.raise("pbeam.bundle: the project config names no directory for the release #{app}")
    end

    Path.expand(path, cwd)
  end

  defp default_output(release, app) do
    Path.join(Path.dirname(release), "#{app}.pbeamb")
  end

  # Cargo reports the path of each artifact, so a configured target
  # directory or build target cannot move or hide the tool.
  defp build_tool!(root) do
    {output, status} =
      System.cmd(
        "cargo",
        ["build", "--locked", "--release", "--bin", "pbeam_bundle", "--message-format=json"],
        cd: root
      )

    if status != 0 do
      Mix.raise("pbeam.bundle: cargo failed with status #{status}")
    end

    output
    |> String.split("\n", trim: true)
    |> Enum.find_value(fn line ->
      case JSON.decode(line) do
        {:ok,
         %{
           "reason" => "compiler-artifact",
           "target" => %{"name" => "pbeam_bundle"},
           "executable" => executable
         }}
        when is_binary(executable) ->
          executable

        _ ->
          nil
      end
    end)
    |> case do
      nil -> Mix.raise("pbeam.bundle: cargo reported no pbeam_bundle artifact")
      executable -> executable
    end
  end

  defp run!(command, arguments, root) do
    {output, status} = System.cmd(command, arguments, cd: root, stderr_to_stdout: true)

    if status != 0 do
      Mix.raise("pbeam.bundle: #{command} failed with status #{status}:\n#{output}")
    end

    output
  end
end
