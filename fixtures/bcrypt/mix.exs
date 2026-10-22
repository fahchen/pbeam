defmodule BcryptFixture.MixProject do
  use Mix.Project

  # The project fetches bcrypt_elixir from Hex at the version of mix.lock and builds its NIF library for tests/feature/nif_bcrypt.rs. `BcryptFixture.main/0` is the entry of the bundle of its release.
  def project do
    [
      app: :bcrypt_fixture,
      version: "0.1.0",
      elixir: "~> 1.20",
      test_coverage: [summary: [threshold: 100]],
      deps: [{:bcrypt_elixir, "3.3.2"}]
    ]
  end
end
