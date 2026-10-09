defmodule Fairness do
  @moduledoc """
  Two busy processes share one scheduler. Neither process waits in a
  receive, but the scheduler switches between them when a process uses its
  reductions, so both make progress.
  """

  @steps 20_000
  @every 10

  @doc """
  Start the busy processes `a` and `b`, collect their progress reports, and
  print one line for each run of reports from one process.
  """
  def main do
    collector = self()
    spawn(fn -> work(collector, :a, 1) end)
    spawn(fn -> work(collector, :b, 1) end)

    collect([], 2)
    |> print(%{a: 0, b: 0})
  end

  # Count to @steps with no receive, and send the name to the collector
  # each @every steps. The loop allocates no heap data, so no garbage
  # collection changes the reductions of the process.
  defp work(collector, _name, step) when step > @steps, do: send(collector, :done)

  defp work(collector, name, step) do
    if rem(step, @every) == 0, do: send(collector, name)
    work(collector, name, step + 1)
  end

  defp collect(reports, 0), do: :lists.reverse(reports)

  defp collect(reports, running) do
    receive do
      :done -> collect(reports, running - 1)
      name -> collect([name | reports], running)
    end
  end

  defp print([], _steps), do: :ok

  defp print([name | _] = reports, steps) do
    {count, rest} = run(reports, name, 0)
    first = Map.fetch!(steps, name) + @every
    last = Map.fetch!(steps, name) + count * @every

    IO.puts([
      Atom.to_charlist(name),
      " ",
      Integer.to_charlist(first),
      "..",
      Integer.to_charlist(last)
    ])

    print(rest, Map.put(steps, name, last))
  end

  defp run([name | rest], name, count), do: run(rest, name, count + 1)
  defp run(rest, _name, count), do: {count, rest}
end
