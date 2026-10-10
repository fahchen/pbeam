defmodule PingPong do
  @rounds 3

  def main do
    parent = self()
    pong = spawn(fn -> pong(@rounds) end)
    spawn(fn -> ping(parent, pong, 1, []) end)

    receive do
      {:done, count, order} ->
        Enum.each(order, &IO.puts/1)
        IO.puts("count: " <> Integer.to_string(count))
    end
  end

  defp ping(parent, _pong, round, order) when round > @rounds do
    send(parent, {:done, round - 1, Enum.reverse(order)})
  end

  defp ping(parent, pong, round, order) do
    send(pong, {:ping, self()})

    receive do
      {:pong, ^pong} ->
        number = Integer.to_string(round)
        ping(parent, pong, round + 1, ["pong " <> number, "ping " <> number | order])
    end
  end

  defp pong(0), do: :ok

  defp pong(rounds) do
    receive do
      {:ping, from} ->
        send(from, {:pong, self()})
        pong(rounds - 1)
    end
  end
end
