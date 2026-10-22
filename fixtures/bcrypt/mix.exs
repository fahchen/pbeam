defmodule BcryptFixture.MixProject do
  use Mix.Project

  # The project has no code of its own. It fetches bcrypt_elixir from Hex at the version of mix.lock and builds its NIF library for tests/integration/nif_bcrypt.rs.
  def project do
    [
      app: :bcrypt_fixture,
      version: "0.1.0",
      elixir: "~> 1.20",
      deps: [{:bcrypt_elixir, "3.3.2"}]
    ]
  end
end
