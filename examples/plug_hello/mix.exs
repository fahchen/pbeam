defmodule PlugHello.MixProject do
  use Mix.Project

  def project do
    [
      app: :plug_hello,
      version: "0.1.0",
      elixir: "~> 1.20",
      start_permanent: Mix.env() == :prod,
      # The Mix task is build tooling: `tests/feature/bundle.rs` runs it end to end, so the application coverage leaves it out.
      test_coverage: [summary: [threshold: 100], ignore_modules: [Mix.Tasks.Pbeam.Bundle]],
      deps: [{:bandit, "~> 1.12"}, {:plug, "~> 1.20"}],
      # The release goes to the build directory of the repository examples, which is not committed.
      releases: [plug_hello: [path: "../../target/examples/plug_hello"]]
    ]
  end

  def application do
    [mod: {PlugHello.Application, []}, extra_applications: [:logger]]
  end
end
