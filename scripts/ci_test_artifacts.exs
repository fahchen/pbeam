defmodule CITestArtifacts do
  @archive "target/ci-fixtures.tar.zst"
  @paths ~w(fixtures/hello/_build/test/lib/hello_reference/ebin target/examples/plug_hello examples/plug_hello/deps)

  def run(["pack"]) do
    verify_fixtures!()
    tar!(~w(--create --zstd --file) ++ [@archive, "--" | @paths])
  end

  def run(["unpack"]) do
    tar!(~w(--extract --zstd --same-permissions --file) ++ [@archive, "--" | @paths])
    verify_fixtures!()
  end

  def run(["restore-rust", archive]) do
    workspace = File.cwd!()
    prepare_rust_target!(workspace)

    command!(
      "cargo-nextest",
      ["nextest", "list", "--archive-file", archive, "--extract-to", workspace] ++
        ~w(--extract-overwrite --workspace-remap) ++ [workspace, "--list-type", "binaries-only"]
    )
  end

  def run(_args),
    do: raise("usage: elixir scripts/ci_test_artifacts.exs pack|unpack|restore-rust ARCHIVE")

  def prepare_rust_target!(workspace) do
    target = Path.join(workspace, "target")

    # Nextest rejects archive paths that traverse a symlink outside the worktree.
    case File.lstat(target) do
      {:ok, %File.Stat{type: :symlink}} ->
        IO.puts("Remove the runner target symlink before restoring the test archive")
        File.rm!(target)

      {:ok, %File.Stat{type: :directory}} ->
        :ok

      {:error, :enoent} ->
        :ok

      other ->
        raise "cannot prepare the Rust test target: #{inspect(other)}"
    end
  end

  def verify_fixtures! do
    for path <- @paths do
      unless File.dir?(path), do: raise("missing fixture directory: #{path}")
    end
  end

  defp tar!(args), do: command!("tar", args)

  defp command!(program, args) do
    case System.cmd(program, args, into: IO.stream(:stdio, :line)) do
      {_, 0} -> :ok
      {_, status} -> raise "#{program} failed with status #{status}"
    end
  end
end

if Path.expand(List.to_string(List.first(:init.get_plain_arguments(), ~c""))) == __ENV__.file do
  CITestArtifacts.run(System.argv())
end
