defmodule RestrictedGuest do
  @workers 3

  # Start each worker with `spawn/3` of a module of the guest, so the
  # `modules` constraint of the policy compares the worker module.
  def main do
    Enum.each(1..@workers, fn number ->
      worker = spawn(RestrictedGuest.Worker, :run, [self(), number])

      receive do
        {:done, ^worker} -> :ok
      end
    end)

    IO.puts("main: " <> Integer.to_string(@workers) <> " workers done")
  end
end

defmodule RestrictedGuest.Worker do
  def run(parent, number) do
    IO.puts("worker " <> Integer.to_string(number) <> ": hello")
    send(parent, {:done, self()})
  end
end
