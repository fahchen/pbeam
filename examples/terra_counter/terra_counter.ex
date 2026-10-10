defmodule TerraCounter do
  def main do
    # `mix run` starts the elixir application, whose start sets standard_io to
    # binary mode (Elixir 1.20.4 lib/elixir/src/elixir.erl:36). Terra reads
    # keys with IO.binread(:stdio, 1) and accepts only a binary, so this entry
    # sets the same option before it runs the app.
    :ok = :io.setopts(:standard_io, [:binary])
    IO.inspect(Terra.run(Counter))
  end
end
