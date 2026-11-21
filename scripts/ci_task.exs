defmodule CITask do
  # The runtime and integration tests read this Markdown file with include_str!.
  @code_docs ["docs/effect-inventory.md"]

  @otp_beam_inputs [
    ".gitmodules",
    "mise.toml",
    "vendor/otp",
    "scripts/otp_suite_beam.exs",
    "tests/otp/cases.txt"
  ]

  def run([]) do
    paths = IO.read(:stdio, :eof)

    paths =
      if paths == :eof, do: [], else: paths |> String.trim_trailing("\n") |> String.split("\n")

    IO.puts(mise_task(paths))
  end

  def run(["--github"]) do
    {task, otp_beams} =
      if System.get_env("EVENT") == "pull_request" do
        paths = changed_paths(System.fetch_env!("BASE"), System.fetch_env!("HEAD"))
        IO.puts("Changed files:\n" <> Enum.map_join(paths, "\n", &inspect/1))
        {mise_task(paths), Enum.any?(paths, &otp_beam_input?/1)}
      else
        {"check", true}
      end

    IO.puts("Task: #{task}")

    File.write!(System.fetch_env!("GITHUB_OUTPUT"), "task=#{task}\notp_beams=#{otp_beams}\n", [
      :append
    ])
  end

  def run(_), do: raise(ArgumentError, "usage: elixir scripts/ci_task.exs [--github]")

  def changed_paths(base, head) do
    # Include both paths of a rename so a source file moved to Markdown still needs checks.
    args = ["diff", "--no-renames", "--name-only", "-z", "#{base}...#{head}", "--"]

    case System.cmd("git", args) do
      {output, 0} -> :binary.split(output, <<0>>, [:global, :trim])
      {_, status} -> raise "git diff failed with status #{status}"
    end
  end

  defp mise_task(paths) do
    if paths != [] and
         Enum.all?(paths, &(String.ends_with?(&1, ".md") and &1 not in @code_docs)) do
      "lint"
    else
      "check"
    end
  end

  defp otp_beam_input?(path) do
    path in @otp_beam_inputs or String.starts_with?(path, "tests/otp/beam/")
  end
end

if Path.expand(List.to_string(List.first(:init.get_plain_arguments(), ~c""))) == __ENV__.file do
  CITask.run(System.argv())
end
