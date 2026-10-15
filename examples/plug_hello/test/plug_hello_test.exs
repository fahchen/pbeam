defmodule PlugHelloTest do
  use ExUnit.Case, async: true
  import Plug.Test

  test "GET / answers Hello world! as text/plain" do
    conn = PlugHello.call(conn(:get, "/"), PlugHello.init([]))
    assert conn.status == 200
    assert conn.resp_body == "Hello world!"
    assert ["text/plain" <> _] = Plug.Conn.get_resp_header(conn, "content-type")
  end

  test "another path answers 404" do
    assert PlugHello.call(conn(:get, "/other"), []).status == 404
  end

  test "the started Bandit listener serves the page over TCP" do
    [{_, listener, _, _}] = Supervisor.which_children(PlugHello.Supervisor)
    {:ok, {_address, port}} = ThousandIsland.listener_info(listener)
    {:ok, socket} = :gen_tcp.connect(~c"127.0.0.1", port, [:binary, active: false])
    :ok = :gen_tcp.send(socket, "GET / HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n")
    assert {:ok, response} = receive_all(socket, "")
    assert response =~ ~r/\AHTTP\/1\.1 200 OK\r\n/
    assert String.ends_with?(response, "\r\n\r\nHello world!")
  end

  defp receive_all(socket, acc) do
    case :gen_tcp.recv(socket, 0, 5_000) do
      {:ok, data} -> receive_all(socket, acc <> data)
      {:error, :closed} -> {:ok, acc}
    end
  end
end
