defmodule FsNotifyFixture do
  @moduledoc """
  The entry of the bundle run of `tests/feature/nif_fs_notify.rs`: the release of the project ships the fs_notify NIF library, and the launcher calls `main/0` with `-s`.
  """

  @doc """
  Watches a new temporary directory, writes a file in it, waits for the first event of the file, stops the watch, and writes the event as one line: its kind, its detail, and the base names of its paths.
  """
  def main do
    directory =
      Path.join(System.tmp_dir!(), "fs_notify_fixture_#{System.unique_integer([:positive])}")

    File.mkdir_p!(directory)
    {:ok, watch} = FsNotify.watch(directory)
    File.write!(Path.join(directory, "hello.txt"), "hello")
    # The first event of the file: an event of the new directory can come
    # first, as the backend of the host reports it. The wait has no timeout:
    # the test of the run has its own time limit.
    event =
      receive do
        {:fs_notify_event, %FsNotify.Event{kind: kind, detail: detail, paths: [path | _] = paths}}
        when byte_size(path) >= 9 and binary_part(path, byte_size(path), -9) == "hello.txt" ->
          {kind, detail, Enum.map(paths, &Path.basename/1)}
      end

    unwatched = FsNotify.unwatch(watch)
    File.rm_rf!(directory)
    IO.inspect({event, unwatched}, width: :infinity)
    :ok
  end
end
