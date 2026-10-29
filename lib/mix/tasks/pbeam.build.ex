defmodule Mix.Tasks.Pbeam.Build do
  @moduledoc """
  Builds the one-file PBeam executable of the release.

  A project that has `pbeam` as a dependency runs this task with
  `MIX_ENV=prod`. The task runs `mix release` for the release of the
  project config, unless `--release` names a built release directory. It
  stops with an error when the ERTS version of the release
  (`releases/start_erl.data`) is not the ERTS version of the Erlang that
  runs the command: the pinned OTP of the PBeam source, whose modules
  PBeam runs. The task builds the `pbeam` runtime and the `pbeam_bundle`
  tool with Cargo in the PBeam source of the dependency, writes the
  bundle with `mix pbeam.bundle`, and appends the bundle to the runtime.
  The default output is `<app>.bundle` next to the release directory.
  Relative paths are relative to the directory of the command line.

      MIX_ENV=prod mix pbeam.build
      MIX_ENV=prod mix pbeam.build --output target/plug_hello --runtime target/release/pbeam --tool target/release/pbeam_bundle --release target/examples/plug_hello
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

    # With no `--release`, the task builds the release of the project
    # config itself. It builds only the release of the prod environment,
    # the configuration that ships, so no development configuration goes
    # into an executable. `--overwrite` builds again when the release
    # directory already has a build.
    unless Keyword.get(options, :release) do
      if Mix.env() != :prod do
        Mix.raise("pbeam.build: run the task with MIX_ENV=prod, the environment of the release")
      end

      Mix.Task.rerun("release", ["--overwrite"])
    end

    release = Bundle.release_directory!(app, options)

    # A release of another ERTS version holds OTP modules that PBeam does
    # not run.
    Bundle.check_erts_version!(release)

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
