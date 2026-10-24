defmodule Mix.Tasks.Pbeam.Build do
  @moduledoc """
  Builds the one-file PBeam executable of the release.

  A project that has `pbeam` as a dependency runs this task. The task builds
  the `pbeam` runtime and the `pbeam_bundle` tool with Cargo in the PBeam
  source of the dependency, writes the bundle with `mix pbeam.bundle`, and
  appends the bundle to the runtime. Run `mix release` first. The default
  output is `<app>.bundle` next to the release directory. Relative paths are
  relative to the directory of the command line.

      mix pbeam.build
      mix pbeam.build --output target/plug_hello --runtime target/release/pbeam --tool target/release/pbeam_bundle
  """

  use Mix.Task

  alias Mix.Tasks.Pbeam.Bundle

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
    app = Mix.Project.config()[:app]
    release = Bundle.release_directory!(app, options)

    output =
      Path.expand(
        Keyword.get(options, :output) || Path.join(Path.dirname(release), "#{app}.bundle"),
        cwd
      )

    bundle = Path.join(Path.dirname(output), "#{app}.pbeamb")
    File.mkdir_p!(Path.dirname(output))

    tool =
      case Keyword.get(options, :tool) do
        nil -> Bundle.cargo_bin!("pbeam_bundle")
        tool -> Path.expand(tool, cwd)
      end

    runtime =
      case Keyword.get(options, :runtime) do
        nil -> Bundle.cargo_bin!("pbeam")
        runtime -> Path.expand(runtime, cwd)
      end

    bundle_options =
      ["--output", bundle, "--tool", tool, "--release", release] ++
        pass(options, :start_list, "--start-list")

    Bundle.run(bundle_options)
    Bundle.run!(tool, ["executable", runtime, bundle, output])
    Mix.shell().info("Wrote #{output}")
  end

  defp pass(options, key, flag) do
    case Keyword.get(options, key) do
      nil -> []
      value -> [flag, value]
    end
  end
end
