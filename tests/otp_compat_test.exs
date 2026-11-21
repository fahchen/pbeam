ExUnit.start()
Code.require_file("../scripts/otp_compat.exs", __DIR__)

defmodule OTPCompatTest do
  use ExUnit.Case, async: true

  import ExUnit.CaptureIO

  @first "example_SUITE:first"
  @second "example_SUITE:second"
  @manifest "tests/otp/cases.txt"
  @baseline "tests/otp/known_failures.txt"

  setup do
    root =
      Path.join(
        System.tmp_dir!(),
        "otp-compat-test-" <> Base.encode16(:crypto.strong_rand_bytes(12))
      )

    beam = Path.join(root, "tests/otp/beam")
    File.mkdir_p!(beam)
    on_exit(fn -> File.rm_rf!(root) end)

    source = """
    -module(example_SUITE).
    -export([first/1, second/1]).
    first([]) -> ok.
    second([]) -> 127.
    """

    source_path = Path.join(root, "example_SUITE.erl")
    File.write!(source_path, source)
    {:ok, :example_SUITE, binary} = :compile.file(String.to_charlist(source_path), [:binary])
    File.write!(Path.join(beam, "example_SUITE.beam"), binary)
    File.write!(Path.join(root, @manifest), @first <> "\n" <> @second <> "\n")
    File.write!(Path.join(root, @baseline), @first <> "\n")

    otp_root = :code.root_dir() |> List.to_string()
    release = :erlang.system_info(:otp_release) |> List.to_string()
    pin = File.read!(Path.join([otp_root, "releases", release, "OTP_VERSION"])) |> String.trim()
    File.write!(Path.join(root, "mise.toml"), ~s(erlang = "#{pin}"\n))

    pbeam = Path.join(root, "pbeam")

    File.write!(pbeam, ~S'''
    #!/usr/bin/env elixir
    expression = List.last(System.argv())
    cond do
      String.contains?(expression, "'example_SUITE':'first'(") -> IO.write(File.read!("first"))
      String.contains?(expression, "'example_SUITE':'second'(") -> IO.write(File.read!("second"))
      true -> System.halt(3)
    end
    ''')

    File.chmod!(pbeam, 0o700)
    results(root, "{return,ok}", "{return,127}")

    %{root: root, pbeam: pbeam}
  end

  test "manifests reject invalid and duplicate case IDs", context do
    for value <- ["invalid", @first <> "\n" <> @first] do
      write(context.root, @manifest, value)
      assert command(context, "check", 1) =~ "invalid or duplicate case ID"
    end
  end

  test "initial Git baseline must be empty and partial baselines fail", context do
    root = context.root
    git(root, ["init", "--quiet"])
    initial = commit(root, ["mise.toml"])

    assert command(context, "check", 1, ["--base", initial]) =~
             "initial known-failure baseline must be empty"

    write(root, @baseline, "")
    assert command(context, "check", 0, ["--base", initial]) == ""

    partial = commit(root, [@manifest])

    assert command(context, "check", 1, ["--base", partial]) =~
             "incomplete OTP compatibility baseline"
  end

  test "Git baseline can expand selection and shrink failures only", context do
    root = context.root
    git(root, ["init", "--quiet"])
    write(root, @manifest, @first <> "\n")
    base = commit(root, [@manifest, @baseline])
    write(root, @manifest, @first <> "\n" <> @second <> "\n")
    write(root, @baseline, "")
    assert command(context, "check", 0, ["--base", base]) == ""

    write(root, @baseline, @second <> "\n")
    assert command(context, "check", 1, ["--base", base]) =~ "added known failures: #{@second}"

    write(root, @manifest, @second <> "\n")
    assert command(context, "check", 1, ["--base", base]) =~ "removed selected cases: #{@first}"
  end

  test "check excludes known cases and requires exact normal returns", context do
    for outcome <- ["{return,ok}", "{skip,unsupported}", "{fail,unsupported}"] do
      results(context.root, :missing, outcome)
      assert command(context, "check", 1) == ""
    end

    results(context.root, :missing, "{return,127}")
    assert command(context, "check", 0) == ""
    assert File.read!(Path.join(context.root, @baseline)) == @first <> "\n"
  end

  test "update requires results for all cases and removes fixed failures only", context do
    results(context.root, "{exception,{error,badarg}}", "{return,127}")
    assert command(context, "update", 0) == ""
    assert File.read!(Path.join(context.root, @baseline)) == @first <> "\n"

    results(context.root, "{return,ok}", "{return,127}")
    assert command(context, "update", 0) == ""
    assert File.read!(Path.join(context.root, @baseline)) == ""
  end

  test "update preserves the baseline on a new failure or an unusable result", context do
    results(context.root, "{return,ok}", "{skip,unsupported}")
    assert command(context, "update", 1) == ""

    for outcome <- [
          :missing,
          "bad",
          "{return,ok,extra}",
          "{return,ok}\nPBEAM_OTP_RESULT {return,ok}"
        ] do
      results(context.root, outcome, "{return,127}")
      assert command(context, "update", 1) =~ "missing or malformed case result"
      assert File.read!(Path.join(context.root, @baseline)) == @first <> "\n"
    end
  end

  test "mismatched OTP pin fails before running the cases", context do
    write(context.root, "mise.toml", ~s(erlang = "0.0.0"\n))
    assert command(context, "check", 1) =~ "use the OTP pin in mise.toml"
  end

  test "update preserves the baseline while another process holds the lock", context do
    baseline = Path.join(context.root, @baseline)
    File.mkdir!(baseline <> ".lock")

    assert command(context, "update", 1) =~ "file already exists"
    assert File.read!(baseline) == @first <> "\n"
    assert File.dir?(baseline <> ".lock")
  end

  test "success marker cannot hide a process error or timeout", %{root: root} do
    marker = ~s|IO.puts("PBEAM_OTP_RESULT {return,ok}"); |

    fail = fn expression, timeout ->
      OTPCompat.run_command(
        ["elixir", "--erl", "+S 1", "-e", marker <> expression],
        "case",
        root,
        [],
        timeout
      )
    end

    assert_raise ArgumentError, ~r/exited with status 7.*PBEAM_OTP_RESULT/s, fn ->
      fail.("System.halt(7)", 5_000)
    end

    assert_raise ArgumentError, ~r/timed out.*PBEAM_OTP_RESULT/s, fn ->
      fail.("Process.sleep(30_000)", 2_000)
    end
  end

  defp write(root, path, content), do: File.write!(Path.join(root, path), content)

  defp results(root, first, second) do
    for {file, outcome} <- [{"first", first}, {"second", second}] do
      value =
        if outcome == :missing, do: "runtime stopped\n", else: "PBEAM_OTP_RESULT #{outcome}\n"

      write(root, file, value)
    end
  end

  defp command(%{root: root, pbeam: pbeam}, mode, expected, extra \\ []) do
    capture_io(:stderr, fn ->
      capture_io(fn ->
        assert OTPCompat.main([mode, "--pbeam", pbeam] ++ extra, root) == expected
      end)
    end)
  end

  defp commit(root, files) do
    git(root, ["add", "--" | files])

    git(root, [
      "-c",
      "user.name=Test",
      "-c",
      "user.email=test@example.com",
      "commit",
      "--no-gpg-sign",
      "--quiet",
      "-m",
      "baseline"
    ])

    git(root, ["rev-parse", "HEAD"]) |> String.trim()
  end

  defp git(root, args),
    do: OTPCompat.run_command(["git" | args], "test repository", root, [], 5_000)
end
