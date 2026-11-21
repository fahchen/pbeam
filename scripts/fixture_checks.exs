defmodule FixtureChecks do
  def run do
    mode = System.get_env("PBEAM_FIXTURE_MODE")

    case mode do
      mode when mode in [nil, "", "full"] ->
        for suite <- ~w(ci_test_artifacts test_selection otp_compat test_classes) do
          run_command("elixir", ["tests/#{suite}_test.exs"], [])
        end

        test(".")
        test("fixtures/hello")
        plug(:full)

        for fixture <- ["bcrypt", "exqlite", "fs_notify"] do
          path = Path.join("fixtures", fixture)
          mix(path, ~w(deps.get --check-locked))
          compile(path)
          test(path)
        end

      "prepare" ->
        compile("fixtures/hello")
        plug(:prepare)

      mode ->
        IO.puts(:stderr, "unknown PBEAM_FIXTURE_MODE: #{mode}")
        System.halt(1)
    end
  end

  defp plug(mode) do
    path = "examples/plug_hello"
    mix(path, ~w(local.hex --force --if-missing))
    mix(path, ~w(local.rebar --force --if-missing))
    mix(path, ~w(deps.get --check-locked))

    if mode == :full do
      compile(path)
      test(path)
    end

    mix(path, ~w(release --overwrite), [{"MIX_ENV", "prod"}])
  end

  defp compile(path), do: mix(path, ~w(compile --warnings-as-errors), [{"MIX_ENV", "test"}])
  defp test(path), do: mix(path, ~w(test --cover --warnings-as-errors))

  defp mix(path, args, env \\ []) do
    run_command("mix", args, cd: path, env: env)
  end

  defp run_command(executable, args, options) do
    {_, status} =
      System.cmd(executable, args, Keyword.put(options, :into, IO.stream(:stdio, :line)))

    if status != 0, do: System.halt(status)
  end
end

FixtureChecks.run()
