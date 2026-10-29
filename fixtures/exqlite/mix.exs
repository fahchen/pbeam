defmodule ExqliteFixture.MixProject do
  use Mix.Project

  # The project fetches exqlite from Hex at the version of mix.lock and builds its NIF library from source (config/config.exs) for tests/feature/nif_exqlite.rs. `ExqliteFixture.main/0` is the entry of the bundle of its release.
  def project do
    [
      app: :exqlite_fixture,
      version: "0.1.0",
      elixir: "~> 1.20",
      test_coverage: [summary: [threshold: 100]],
      deps: [{:exqlite, "0.37.0"}]
    ]
  end
end
