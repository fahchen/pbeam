defmodule CheckBrief do
  @log "target/check.log"
  @flake "restores_the_terminal_before_the_result_of_terra_counter_as_interactive_erts"
  @flake_message "erl_child_setup: failed with error 32"
  @task_failure ~r/^\[([^\]]*)\] ERROR task failed$/
  @progress ~r/^\s*$|^\s*(Compiling|Checking|Running|PASS|START) |^\s*running [0-9]+ tests?$|^\s*test .* \.\.\. ok$|^\s*test result: ok\.|^ERROR task failed$/

  def run do
    File.mkdir_p!("target")
    File.write!(@log, "")

    {_, status} =
      System.cmd("mise", ["run", "check"],
        stderr_to_stdout: true,
        into: File.stream!(@log, [:append])
      )

    status =
      if status != 0 and only_known_flake?() do
        retry_flake()
      else
        status
      end

    if status == 0 do
      IO.puts("check: pass")
    else
      IO.puts("check: FAIL (exit #{status}; full log: #{@log})")
      lines = clean_lines()

      for task <- failed_tasks(lines) do
        lines
        |> task_lines(task)
        |> Enum.reduce([], fn line, step ->
          if String.starts_with?(line, "$ "), do: [line], else: [line | step]
        end)
        |> Enum.reverse()
        |> Enum.reject(&Regex.match?(@progress, &1))
        |> Enum.take(60)
        |> Enum.each(&IO.puts/1)
      end

      System.halt(status)
    end
  end

  defp clean_lines do
    root = File.cwd!() <> "/"

    @log
    |> File.stream!()
    |> Enum.map(fn line ->
      line
      |> String.trim_trailing("\n")
      |> String.replace(~r/\e(\[[0-9;]*[A-Za-z]|\([A-Z0-9])/, "")
      |> String.replace(root, "")
    end)
  end

  defp failed_tasks(lines) do
    for line <- lines, [_, task] <- [Regex.run(@task_failure, line)], do: task
  end

  defp task_lines(lines, task) do
    prefix = "[#{task}] "

    for line <- lines, String.starts_with?(line, prefix) do
      String.replace_prefix(line, prefix, "")
    end
  end

  # The pinned ERTS process can fail with EPIPE in this pty test under host load (#642).
  defp only_known_flake? do
    lines = clean_lines()
    test_lines = task_lines(lines, "test")

    final_failures =
      test_lines
      |> Enum.drop_while(&(not Regex.match?(~r/^ +Summary \[/, &1)))
      |> Enum.drop(1)
      |> Enum.filter(&Regex.match?(~r/^ +(?!SLOW )[A-Z]/, &1))

    failed_tasks(lines) == ["test"] and
      Enum.any?(test_lines, &String.contains?(&1, @flake_message)) and
      case final_failures do
        [failure] ->
          fields = String.split(failure)

          List.first(fields) == "FAIL" and
            Enum.take(fields, -2) == ["pbeam-runtime::example", @flake]

        _ ->
          false
      end
  end

  defp retry_flake do
    message = "check: retried #{@flake} after `#{@flake_message}`"
    IO.puts(message)
    File.write!(@log, message <> "\n", [:append])

    {output, status} =
      System.cmd(
        "cargo",
        ~w(nextest run --locked --workspace --all-features --status-level slow --final-status-level slow --no-fail-fast) ++
          ["-E", "test(=#{@flake})"],
        stderr_to_stdout: true
      )

    lines =
      output
      |> String.split("\n")
      |> Enum.drop(if String.ends_with?(output, "\n"), do: -1, else: 0)

    retry_output = for line <- lines, output != "", do: ["[retry] ", line, "\n"]
    File.write!(@log, retry_output, [:append])
    if status != 0, do: File.write!(@log, "[retry] ERROR task failed\n", [:append])
    status
  end
end

CheckBrief.run()
