defmodule Mix.Tasks.Pbeam.Build do
  @moduledoc """
  Builds the one-file PBeam executable of the release.

  The task builds the `pbeam` runtime and the `pbeam_bundle` tool with
  Cargo, writes the bundle with `mix pbeam.bundle`, and appends the bundle
  to the runtime. Run `mix release` first. Relative paths are relative to
  the directory of the command line.

      mix pbeam.build
      mix pbeam.build --output target/plug_hello --runtime target/release/pbeam --tool target/release/pbeam_bundle
  """

  use Mix.Task

  @shortdoc "Builds the one-file PBeam executable of the release"

  @impl Mix.Task
  def run(arguments) do
    {options, positional, invalid} =
      OptionParser.parse(arguments,
        strict: [
          output: :string,
          runtime: :string,
          tool: :string,
          release: :string,
          start_list: :string
        ]
      )

    if invalid != [] or positional != [] do
      Mix.raise("pbeam.build: unknown arguments #{inspect(invalid ++ positional)}")
    end

    cwd = File.cwd!()
    root = Path.expand("../..", cwd)
    app = Mix.Project.config()[:app]
    output = Path.expand(Keyword.get(options, :output) || default_output(root, app), cwd)
    bundle = Path.join(Path.dirname(output), "#{app}.pbeamb")
    File.mkdir_p!(Path.dirname(output))

    tool =
      case Keyword.get(options, :tool) do
        nil -> cargo_bin!(root, "pbeam_bundle")
        tool -> Path.expand(tool, cwd)
      end

    runtime =
      case Keyword.get(options, :runtime) do
        nil -> cargo_bin!(root, "pbeam")
        runtime -> Path.expand(runtime, cwd)
      end

    bundle_options =
      ["--output", bundle, "--tool", tool] ++
        pass(options, :release, "--release") ++ pass(options, :start_list, "--start-list")

    Mix.Tasks.Pbeam.Bundle.run(bundle_options)
    run!(tool, ["executable", runtime, bundle, output], root)
    Mix.shell().info("Wrote #{output}")
  end

  defp default_output(root, app) do
    Path.join([root, "target/examples", "#{app}.bundle"])
  end

  defp pass(options, key, flag) do
    case Keyword.get(options, key) do
      nil -> []
      value -> [flag, value]
    end
  end

  # Cargo reports the path of each artifact, so a configured target
  # directory or build target cannot move or hide the binary.
  defp cargo_bin!(root, name) do
    {output, status} =
      System.cmd(
        "cargo",
        ["build", "--locked", "--release", "--bin", name, "--message-format=json"],
        cd: root
      )

    if status != 0 do
      Mix.raise("pbeam.build: cargo failed with status #{status}")
    end

    output
    |> String.split("\n", trim: true)
    |> Enum.find_value(fn line ->
      case JSON.decode(line) do
        {:ok,
         %{
           "reason" => "compiler-artifact",
           "target" => %{"name" => ^name},
           "executable" => executable
         }}
        when is_binary(executable) ->
          executable

        _ ->
          nil
      end
    end)
    |> case do
      nil -> Mix.raise("pbeam.build: cargo reported no #{name} artifact")
      executable -> executable
    end
  end

  defp run!(command, arguments, root) do
    {output, status} = System.cmd(command, arguments, cd: root, stderr_to_stdout: true)

    if status != 0 do
      Mix.raise("pbeam.build: #{command} failed with status #{status}:\n#{output}")
    end
  end
end
