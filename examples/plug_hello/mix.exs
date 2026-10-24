defmodule PlugHello.MixProject do
  use Mix.Project

  def project do
    [
      app: :plug_hello,
      version: "0.1.0",
      elixir: "~> 1.20",
      start_permanent: Mix.env() == :prod,
      test_coverage: [summary: [threshold: 100]],
      # The pbeam dependency gives the tasks `mix pbeam.build` and `mix pbeam.bundle`. The release does not need it.
      deps: [{:bandit, "~> 1.12"}, {:pbeam, path: "../..", runtime: false}, {:plug, "~> 1.20"}],
      # The release goes to the build directory of the repository examples, which is not committed.
      releases: [plug_hello: [path: "../../target/examples/plug_hello"]]
    ]
  end

  def application do
    [mod: {PlugHello.Application, []}, extra_applications: [:logger]]
  end
end
