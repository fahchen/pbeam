defmodule FsNotifyFixture.MixProject do
  use Mix.Project

  # The project fetches fs_notify from Hex at the version of mix.lock and builds its NIF library from source with rustler (config/config.exs) for tests/feature/nif_fs_notify.rs. `FsNotifyFixture.main/0` is the entry of the bundle of its release.
  def project do
    [
      app: :fs_notify_fixture,
      version: "0.1.0",
      elixir: "~> 1.20",
      test_coverage: [summary: [threshold: 100]],
      deps: [{:fs_notify, "0.1.2"}, {:rustler, "~> 0.38.0"}]
    ]
  end
end
