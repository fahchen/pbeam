defmodule PBeam.MixProject do
  use Mix.Project

  def project do
    [
      app: :pbeam,
      version: "0.1.0",
      elixir: "~> 1.20",
      # The Mix tasks are build tooling: `tests/feature/build.rs`, `tests/feature/bundle.rs`, and `tests/feature/serve.rs` run them end to end, so the application coverage leaves them out.
      test_coverage: [
        summary: [threshold: 100],
        ignore_modules: [Mix.Tasks.Pbeam.Bundle, Mix.Tasks.Pbeam.Build]
      ],
      deps: []
    ]
  end
end
