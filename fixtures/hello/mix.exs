defmodule HelloReference.MixProject do
  use Mix.Project

  def project do
    [
      app: :hello_reference,
      version: "0.1.0",
      test_coverage: [summary: [threshold: 100]],
      deps: []
    ]
  end
end
