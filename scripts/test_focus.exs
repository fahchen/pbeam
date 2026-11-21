{output, status} =
  System.cmd("cargo", ["test", "--locked", "--quiet" | System.argv()], stderr_to_stdout: true)

progress =
  ~r/^\s*$|^running [0-9]+ tests?$|^[.Fi]+( [0-9]+\/[0-9]+)?$|^test result: ok\. 0 passed;/

lines = output |> String.split("\n") |> Enum.reject(&Regex.match?(progress, &1))
Enum.each(lines, &IO.puts/1)

if status == 0 and not Enum.any?(lines, &String.starts_with?(&1, "test result: ")) do
  IO.puts("no test ran")
  System.halt(1)
end

System.halt(status)
