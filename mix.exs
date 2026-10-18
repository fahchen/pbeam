defmodule PBeam.MixProject do
  use Mix.Project

  def project do
    [
      app: :pbeam,
      version: "0.1.0",
      elixir: "~> 1.20",
      test_coverage: [summary: [threshold: 100]],
      deps: []
    ]
  end
end
