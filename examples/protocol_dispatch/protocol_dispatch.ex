defmodule ProtocolDispatch do
  defmodule Point do
    defstruct [:x, :y]
  end

  defimpl String.Chars, for: Point do
    def to_string(%Point{x: x, y: y}) do
      "(" <> Integer.to_string(x) <> ", " <> Integer.to_string(y) <> ")"
    end
  end

  defimpl Inspect, for: Point do
    def inspect(%Point{x: x, y: y}, _opts) do
      "#Point<" <> Integer.to_string(x) <> ", " <> Integer.to_string(y) <> ">"
    end
  end

  def main do
    show(%Point{x: 1, y: 2}, :ok)
  end

  # Inspect.inspect/2 is the protocol call of IO.inspect/1, without the Inspect.Algebra formatting.
  defp show(point, atom) do
    IO.puts("String.Chars consolidated: " <> Atom.to_string(Protocol.consolidated?(String.Chars)))
    IO.puts("Inspect consolidated: " <> Atom.to_string(Protocol.consolidated?(Inspect)))
    IO.puts("to_string of a point: #{point}")
    IO.puts("inspect of a point: " <> Inspect.inspect(point, %Inspect.Opts{}))
    IO.puts("to_string of an atom: #{atom}")
  end
end
