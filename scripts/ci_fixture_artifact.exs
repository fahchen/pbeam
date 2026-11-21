defmodule CIFixtureArtifact do
  @archive "target/ci-fixtures.tar.zst"
  @paths ~w(fixtures/hello/_build/test/lib/hello_reference/ebin target/examples/plug_hello examples/plug_hello/deps)

  def run(["pack"]) do
    verify!()
    tar!(~w(--create --zstd --file) ++ [@archive, "--" | @paths])
  end

  def run(["unpack"]) do
    tar!(~w(--extract --zstd --same-permissions --file) ++ [@archive, "--" | @paths])
    verify!()
  end

  def run(_args), do: raise("usage: elixir scripts/ci_fixture_artifact.exs pack|unpack")

  def verify! do
    for path <- @paths do
      unless File.dir?(path), do: raise("missing fixture directory: #{path}")
    end
  end

  defp tar!(args) do
    case System.cmd("tar", args, into: IO.stream(:stdio, :line)) do
      {_, 0} -> :ok
      {_, status} -> raise "tar failed with status #{status}"
    end
  end
end

if Path.expand(List.to_string(List.first(:init.get_plain_arguments(), ~c""))) == __ENV__.file do
  CIFixtureArtifact.run(System.argv())
end
