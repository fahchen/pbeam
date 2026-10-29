defmodule Mix.Tasks.Pbeam.Bundle do
  @moduledoc """
  Writes the PBeam application bundle of the release.

  A project that has `pbeam` as a dependency runs this task. The task builds
  the `pbeam_bundle` tool with Cargo in the PBeam source of the dependency,
  writes the start list (the BEAM files of the preloaded modules), and writes
  one bundle. The entry `elixir:start_cli` is the `-s` call of
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

  # This file is in lib/mix/tasks/ of the PBeam source. A path or a git
  # dependency compiles the file from that source, so the Cargo project is
  # three directories up, whatever the directory of the command line is.
  @source Path.expand("../../..", __DIR__)

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
    app = Mix.Project.config()[:app]
    release = release_directory!(app, options)
    output = Path.expand(Keyword.get(options, :output) || default_output(release, app), cwd)
    File.mkdir_p!(Path.dirname(output))

    start =
      Path.expand(
        Keyword.get(options, :start_list) || Path.join(release, "pbeam.start.list"),
        cwd
      )

    tool =
      case Keyword.get(options, :tool) do
        nil -> cargo_bin!("pbeam_bundle")
        tool -> Path.expand(tool, cwd)
      end

    # ERTS builds the preloaded modules into the emulator; their BEAM files
    # are in the ebin directory of erts. The list has one path on each line.
    ebin = Path.join(:code.lib_dir(:erts), "ebin")
    File.write!(start, Enum.map(:erlang.pre_loaded(), &[Path.join(ebin, "#{&1}.beam"), ?\n]))

    # The NIF manifest gate of the tool prints each NIF function that PBeam
    # does not bind.
    gate = run!(tool, ["bundle", release, output, "elixir", "start_cli", start])
    Mix.shell().info(gate)
    Mix.shell().info("Wrote #{output}")
  end

  # Return the release directory of `--release`, of the project config, or
  # of the `mix release` default: `_build/<env>/rel/<app>` below the build
  # path of the project, where `mix release` assembles a release whose
  # config names no `path`.
  @doc false
  def release_directory!(app, options) do
    default = Path.join([Mix.Project.build_path(), "rel", to_string(app)])

    path =
      Keyword.get(options, :release) || Mix.Project.config()[:releases][app][:path] || default

    Path.expand(path)
  end

  # Stop the task when the ERTS version of the release, the first field of
  # `releases/start_erl.data`, is not the ERTS version of the Erlang that
  # runs the task. PBeam runs the OTP modules of that Erlang: the pinned
  # OTP of the PBeam source (`mise.toml`), which the build host must run.
  # A release of another ERTS version holds OTP modules that PBeam does
  # not run.
  @doc false
  def check_erts_version!(release) do
    data = Path.join(release, "releases/start_erl.data")

    release_erts =
      with {:ok, contents} <- File.read(data),
           [erts | _rest] <- String.split(contents) do
        erts
      else
        _error -> Mix.raise("pbeam: cannot read #{data}: the release names no ERTS version")
      end

    erts = List.to_string(:erlang.system_info(:version))

    if release_erts != erts do
      Mix.raise(
        "pbeam: the release #{release} was built with ERTS #{release_erts}, but PBeam runs " <>
          "the OTP modules of ERTS #{erts} (the pinned OTP #{pinned_otp()} of the PBeam " <>
          "source). Build the release with the pinned OTP."
      )
    end
  end

  # Return the OTP pin of the PBeam source: the `erlang` version of its
  # `mise.toml`, which the Rust build also reads (`otp_release` of
  # `build.rs`).
  defp pinned_otp do
    "erlang = \"" <> pin =
      @source
      |> Path.join("mise.toml")
      |> File.read!()
      |> String.split("\n")
      |> Enum.find(&String.starts_with?(&1, "erlang = \""))

    String.trim_trailing(pin, "\"")
  end

  defp default_output(release, app) do
    Path.join(Path.dirname(release), "#{app}.pbeamb")
  end

  # Build one binary of the PBeam source and return its path. Cargo reports
  # the path of each artifact, so a configured target directory or build
  # target cannot move or hide the binary.
  @doc false
  def cargo_bin!(name) do
    {output, status} =
      System.cmd(
        "cargo",
        ["build", "--locked", "--release", "--bin", name, "--message-format=json"],
        cd: @source
      )

    if status != 0 do
      Mix.raise("pbeam: cargo failed with status #{status}")
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
      nil -> Mix.raise("pbeam: cargo reported no #{name} artifact")
      executable -> executable
    end
  end

  # Run a command and return its output, or stop the task on a failure.
  @doc false
  def run!(command, arguments) do
    {output, status} = System.cmd(command, arguments, stderr_to_stdout: true)

    if status != 0 do
      Mix.raise("pbeam: #{command} failed with status #{status}:\n#{output}")
    end

    output
  end
end
