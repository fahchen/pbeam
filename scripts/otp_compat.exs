defmodule OTPCompat do
  @root Path.expand("..", __DIR__)
  @cases "tests/otp/cases.txt"
  @known_failures "tests/otp/known_failures.txt"
  @beam "tests/otp/beam"
  @result "PBEAM_OTP_RESULT "
  @timeout 60_000

  def main(args, root \\ @root) do
    case OptionParser.parse(args, strict: [base: :string, pbeam: :string]) do
      {options, [mode], []} when mode in ["check", "update"] ->
        if mode == "update" and options[:base] do
          IO.puts(:stderr, "OTP compatibility: --base requires check")
          2
        else
          check(mode, options, root)
        end

      _ ->
        IO.puts(
          :stderr,
          "Usage: elixir scripts/otp_compat.exs check|update [--base REV] [--pbeam PATH]"
        )

        2
    end
  end

  defp check(mode, options, root) do
    cases = read_ids(@cases, File.read!(Path.join(root, @cases)))
    previous = File.read!(Path.join(root, @known_failures))
    known = read_ids(@known_failures, previous)
    if MapSet.size(cases) == 0, do: fail("#{@cases} must select at least one case")
    reject_added(known, cases, "unknown case IDs in the failure list")

    if mode == "check" do
      bases = [options[:base], System.get_env("usage_base"), System.get_env("PBEAM_CI_BASE")]

      if base = Enum.find(bases, &(&1 not in [nil, ""])),
        do: check_baseline(base, cases, known, root)
    end

    directories = suite_paths(cases, root)
    target = System.get_env("CARGO_TARGET_DIR", Path.join(root, "target"))
    pbeam = Path.expand(options[:pbeam] || Path.join(target, "debug/pbeam"))
    env = runtime_env()
    {native, pbeam} = runtime_commands(pbeam, env, root)
    selected = if mode == "check", do: MapSet.difference(cases, known), else: cases

    failures =
      selected
      |> Enum.sort()
      |> Enum.reduce(MapSet.new(), fn case_id, failures ->
        [suite, name] = String.split(case_id, ":")
        directory = Map.fetch!(directories, suite)
        expression = expression(suite, name)
        expected = execute(native, directory, expression, "ERTS #{case_id}", root, env)

        unless String.starts_with?(expected, "{return,") do
          fail("ERTS #{case_id}: expected a normal return, got #{expected}")
        end

        actual = execute(pbeam, directory, expression, "PBeam #{case_id}", root, env)

        if actual == expected do
          IO.puts("PASS #{case_id}")
          failures
        else
          IO.puts("FAIL #{case_id}: expected #{expected}, got #{actual}")
          MapSet.put(failures, case_id)
        end
      end)

    if MapSet.subset?(failures, known) do
      if mode == "update" and failures != known do
        replace_failures(previous, failures, root)
      end

      passed = MapSet.size(selected) - MapSet.size(failures)
      excluded = MapSet.size(cases) - MapSet.size(selected)

      IO.puts(
        "OTP compatibility: #{passed} passed, #{MapSet.size(failures)} failed, #{excluded} excluded"
      )

      0
    else
      1
    end
  rescue
    error in [ArgumentError, File.Error, ErlangError] ->
      IO.puts(:stderr, "OTP compatibility: #{Exception.message(error)}")
      1
  end

  defp read_ids(source, contents) do
    contents
    |> lines()
    |> Enum.with_index(1)
    |> Enum.reduce(MapSet.new(), fn {line, number}, ids ->
      case_id = String.trim(line)

      cond do
        case_id == "" ->
          ids

        Regex.match?(~r/\A[a-z][a-zA-Z0-9_]*_SUITE:[a-z][a-zA-Z0-9_]*\z/, case_id) and
            not MapSet.member?(ids, case_id) ->
          MapSet.put(ids, case_id)

        true ->
          fail("#{source}:#{number}: invalid or duplicate case ID #{inspect(case_id)}")
      end
    end)
  end

  defp check_baseline(revision, cases, known, root) do
    tree =
      run(
        ["git", "rev-parse", "--verify", "--end-of-options", revision <> "^{tree}"],
        "base #{revision}",
        root
      )
      |> String.trim()

    paths =
      run(
        ["git", "ls-tree", "-r", "--name-only", tree, "--", @cases, @known_failures],
        "base #{revision}",
        root
      )

    case paths |> lines() |> Enum.reject(&(&1 == "")) |> Enum.sort() do
      [] ->
        if MapSet.size(known) != 0, do: fail("the initial known-failure baseline must be empty")

      [@cases, @known_failures] ->
        [base_cases, base_known] =
          for path <- [@cases, @known_failures] do
            read_ids(
              "#{revision}:#{path}",
              run(["git", "show", "#{tree}:#{path}"], "base #{revision}:#{path}", root)
            )
          end

        if MapSet.size(base_cases) == 0 or not MapSet.subset?(base_known, base_cases) do
          fail("base #{revision}: invalid OTP compatibility baseline")
        end

        reject_added(base_cases, cases, "removed selected cases")
        reject_added(known, base_known, "added known failures")

      _ ->
        fail("base #{revision}: incomplete OTP compatibility baseline")
    end
  end

  defp reject_added(actual, allowed, message) do
    extra = actual |> MapSet.difference(allowed) |> Enum.sort()
    if extra != [], do: fail("#{message}: #{Enum.join(extra, ", ")}")
  end

  defp suite_paths(cases, root) do
    cases
    |> Enum.map(fn case_id -> hd(String.split(case_id, ":")) end)
    |> Enum.uniq()
    |> Enum.sort()
    |> Map.new(fn suite ->
      matches = Path.wildcard(Path.join([root, @beam, "**", "#{suite}.beam"]))

      case matches do
        [file] -> {suite, Path.dirname(file)}
        _ -> fail("#{suite}: expected one upstream BEAM file, found #{length(matches)}")
      end
    end)
  end

  defp runtime_env do
    unset = ["ERL_AFLAGS", "ERL_FLAGS", "ERL_ZFLAGS", "ERL_LIBS", "ERL_CRASH_DUMP"]

    extra =
      System.get_env()
      |> Map.keys()
      |> Enum.filter(&Regex.match?(~r/\AERL_OTP.*_FLAGS\z/, &1))

    [
      {~c"ERL_CRASH_DUMP_SECONDS", ~c"0"}
      | Enum.map(unset ++ extra, &{String.to_charlist(&1), false})
    ]
  end

  defp runtime_commands(pbeam, env, root) do
    native = ["erl", "+S", "1", "-noshell"]

    probe = ~S'''
    {ok, [[Root]]} = init:get_argument(root),
    {ok, [[Bin]]} = init:get_argument(bindir),
    io:format("~s~n~s~n~s~n~s~n",
      [Root, Bin, code:lib_dir(erts, ebin), erlang:system_info(otp_release)]), halt().
    '''

    {erts_root, bindir, preloaded, release} =
      case lines(run(native ++ ["-eval", probe], "pinned ERTS paths", root, env)) do
        [erts_root, bindir, preloaded, release, ""] ->
          {erts_root, bindir, preloaded, release}

        output ->
          fail("invalid pinned ERTS path output: #{inspect(output)}")
      end

    pin = Regex.run(~r/^erlang = "([^"]+)"$/m, File.read!(Path.join(root, "mise.toml")))

    installed =
      Path.join([erts_root, "releases", release, "OTP_VERSION"]) |> File.read!() |> String.trim()

    unless match?([_, ^installed], pin) do
      fail("erl on PATH is OTP #{installed}; use the OTP pin in mise.toml")
    end

    unless File.regular?(pbeam), do: fail("missing #{pbeam}; build pbeam first")

    {native,
     [
       pbeam,
       "erl",
       "--root",
       erts_root,
       "--bindir",
       bindir,
       "--preloaded",
       preloaded,
       "--",
       "+S",
       "1",
       "-noshell"
     ]}
  end

  defp expression(suite, name) do
    """
    {module, '#{suite}'} = code:ensure_loaded('#{suite}'),
    true = erlang:function_exported('#{suite}', '#{name}', 1),
    Result = try '#{suite}':'#{name}'([]) of
      {skip, Why} -> {skip, Why};
      {fail, Why} -> {fail, Why};
      Value -> {return, Value}
    catch Class:Reason -> {exception, {Class, Reason}} end,
    io:format("~n#{@result}~w~n", [Result]), halt().
    """
  end

  defp execute(command, directory, expression, subject, root, env) do
    output = run(command ++ ["-pa", directory, "-eval", expression], subject, root, env)

    records = for <<@result, record::binary>> <- lines(output), do: record

    case records do
      [record] ->
        if valid_result?(record) do
          record
        else
          malformed_result(subject, output)
        end

      _ ->
        malformed_result(subject, output)
    end
  end

  defp valid_result?(record) do
    with true <- String.valid?(record),
         {:ok, tokens, _} <- :erl_scan.string(String.to_charlist(record <> ".")),
         {:ok, {tag, _value}} when tag in [:return, :skip, :fail, :exception] <-
           :erl_parse.parse_term(tokens) do
      true
    else
      _ -> false
    end
  end

  defp malformed_result(subject, output) do
    fail("#{subject}: missing or malformed case result\n#{diagnostics(output)}")
  end

  defp run(command, subject, root, env \\ []) do
    run_command(command, subject, root, env, @timeout)
  end

  def run_command([name | args], subject, root, env, timeout) do
    executable = System.find_executable(name) || fail("#{subject}: missing executable #{name}")

    port =
      Port.open({:spawn_executable, executable}, [
        :binary,
        :exit_status,
        :stderr_to_stdout,
        :in,
        :hide,
        cd: root,
        env: env,
        args: args
      ])

    deadline = System.monotonic_time(:millisecond) + timeout

    case receive_output(port, deadline, []) do
      {0, output} ->
        output

      {:timeout, output} ->
        case Port.info(port, :os_pid) do
          {:os_pid, pid} ->
            System.cmd("/bin/kill", ["-KILL", Integer.to_string(pid)], stderr_to_stdout: true)
            close_port(port)

          nil ->
            :ok
        end

        fail("#{subject}: timed out after #{timeout}ms\n#{diagnostics(output)}")

      {status, output} ->
        fail("#{subject}: exited with status #{status}\n#{diagnostics(output)}")
    end
  end

  defp close_port(port) do
    Port.close(port)
  rescue
    # A killed process can close its port first.
    ArgumentError -> :ok
  end

  defp receive_output(port, deadline, chunks) do
    remaining = deadline - System.monotonic_time(:millisecond)

    if remaining <= 0 do
      {:timeout, IO.iodata_to_binary(Enum.reverse(chunks))}
    else
      receive do
        {^port, {:data, chunk}} ->
          receive_output(port, deadline, [chunk | chunks])

        {^port, {:exit_status, status}} ->
          {status, IO.iodata_to_binary(Enum.reverse(chunks))}
      after
        remaining -> {:timeout, IO.iodata_to_binary(Enum.reverse(chunks))}
      end
    end
  end

  defp replace_failures(previous, failures, root) do
    destination = Path.join(root, @known_failures)
    lock = destination <> ".lock"
    File.mkdir!(lock)
    temporary = Path.join(lock, "known_failures.txt")

    try do
      if File.read!(destination) != previous do
        fail("#{@known_failures} changed during execution")
      end

      File.write!(temporary, Enum.map(Enum.sort(failures), &(&1 <> "\n")))
      File.rename!(temporary, destination)
    after
      File.rm(temporary)
      File.rmdir(lock)
    end
  end

  defp lines(bytes), do: :binary.split(bytes, ["\r\n", "\n"], [:global])

  defp diagnostics(output) do
    "stdout and stderr: #{inspect(output, printable_limit: :infinity)}"
  end

  defp fail(message), do: raise(ArgumentError, message)
end

if Path.expand(List.to_string(List.first(:init.get_plain_arguments(), ~c""))) == __ENV__.file do
  System.halt(OTPCompat.main(System.argv()))
end
