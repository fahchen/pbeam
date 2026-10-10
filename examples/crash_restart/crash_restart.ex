defmodule CrashRestart do
  @restarts 3

  def main do
    # Each crash also makes ERTS log an error report with a time stamp from another process, so the report changes the output on each run. The example turns off log events to give the same output on each run.
    :logger.set_primary_config(:level, :none)
    Process.flag(:trap_exit, true)
    supervise(start_worker(1), 1)
  end

  defp start_worker(run) do
    IO.puts("start worker " <> Integer.to_string(run))
    spawn_link(fn -> work(run) end)
  end

  defp work(run) do
    raise "worker " <> Integer.to_string(run) <> " crashed"
  end

  defp supervise(pid, run) do
    receive do
      {:EXIT, ^pid, {%RuntimeError{message: message}, _stacktrace}} ->
        IO.puts("exit: " <> message)

        if run <= @restarts do
          IO.puts("restart")
          supervise(start_worker(run + 1), run + 1)
        else
          IO.puts("stop after " <> Integer.to_string(run) <> " crashes")
        end
    end
  end
end
