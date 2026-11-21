defmodule OtpSuiteBeam do
  @otp "vendor/otp"
  @output "tests/otp/beam"
  @sources ~w(
    erts/emulator/test/bs_match_int_SUITE.erl
    erts/emulator/test/binary_SUITE.erl
    lib/stdlib/test/lists_SUITE.erl
  )

  def run(args) when args in [[], ["--write"]] do
    File.cd!(Path.expand("..", __DIR__))

    with {<<"160000 ", pin::binary>>, 0} <-
           System.cmd("git", ["ls-files", "--stage", @otp]),
         [pin, "0", @otp] <- String.split(pin),
         {head, 0} <- System.cmd("git", ["-C", @otp, "rev-parse", "HEAD"], stderr_to_stdout: true),
         true <- String.trim(head) == pin do
      rebuild(pin, args == ["--write"])
    else
      _ ->
        IO.puts(
          :stderr,
          "otp-suite-beam: check out the pinned source with git submodule update --init #{@otp}"
        )

        1
    end
  end

  def run(_) do
    IO.puts(:stderr, "Usage: elixir scripts/otp_suite_beam.exs [--write]")
    2
  end

  defp rebuild(pin, write?) do
    version =
      command!("mise", ["config", "get", "--file", "mise.toml", "tools.erlang"]) |> String.trim()

    otp_root = List.to_string(:code.root_dir())

    installed_version =
      Path.join([otp_root, "releases", :erlang.system_info(:otp_release), "OTP_VERSION"])

    source_version = command!("git", ["-C", @otp, "show", "#{pin}:OTP_VERSION"])

    if String.trim(source_version) == version and
         match?({:ok, ^version}, read_version(installed_version)) do
      scratch = command!("mktemp", ["-d"]) |> String.trim()

      try do
        paths =
          for source <- @sources do
            path = Path.join(scratch, Path.basename(source))
            File.write!(path, command!("git", ["-C", @otp, "show", "#{pin}:#{source}"]))
            path
          end

        command!(
          Path.join(otp_root, "bin/erlc"),
          ["-W0", "+deterministic", "+no_debug_info", "-o", scratch | paths],
          env: [{"ERL_COMPILER_OPTIONS", nil}]
        )

        check_outputs(scratch, version, write?)
      after
        File.rm_rf!(scratch)
      end
    else
      IO.puts(
        :stderr,
        "otp-suite-beam: both the source and the Erlang installation must be OTP-#{version}"
      )

      1
    end
  end

  defp read_version(path) do
    with {:ok, version} <- File.read(path), do: {:ok, String.trim(version)}
  end

  defp check_outputs(scratch, version, write?) do
    expected = Enum.map(@sources, &(Path.rootname(&1, ".erl") <> ".beam"))

    differences =
      for beam <- expected,
          output = Path.join(@output, beam),
          binary = File.read!(Path.join(scratch, Path.basename(beam))),
          !check_output(output, binary, write?),
          do: output

    for output <- differences do
      IO.puts(
        :stderr,
        "otp-suite-beam: #{output} differs from OTP-#{version}; regenerate with --write"
      )
    end

    actual =
      for path <- Path.wildcard("#{@output}/**/*.beam", match_dot: true),
          do: Path.relative_to(path, @output)

    if Enum.sort(actual) == Enum.sort(expected) do
      if differences == [], do: 0, else: 1
    else
      IO.puts(:stderr, "otp-suite-beam: committed BEAM paths must match the upstream source list")
      IO.puts(:stderr, "Missing: #{inspect(expected -- actual)}")
      IO.puts(:stderr, "Extra: #{inspect(actual -- expected)}")
      1
    end
  end

  defp check_output(output, binary, true) do
    File.mkdir_p!(Path.dirname(output))
    File.write!(output, binary)
    true
  end

  defp check_output(output, binary, false), do: File.read(output) == {:ok, binary}

  defp command!(executable, args, opts \\ []) do
    case System.cmd(executable, args, opts) do
      {output, 0} ->
        output

      {output, status} ->
        raise "#{Enum.join([executable | args], " ")} failed (#{status})\n#{output}"
    end
  end
end

args = if System.get_env("usage_write") == "true", do: ["--write"], else: System.argv()
System.halt(OtpSuiteBeam.run(args))
