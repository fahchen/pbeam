defmodule PlugHello do
  @moduledoc """
  The plug that answers `GET /` with `Hello world from PBeam!` and `GET /argv`
  with the arguments of the command line, so a test can read what the
  executable gave the release as `System.argv()`. `GET /release` answers
  with the `RELEASE_*` variables that the release script exports, and
  `GET /executable` with `PBEAM_EXECUTABLE`, the path of the one-file
  executable.
  """

  @release_variables ~w(RELEASE_NAME RELEASE_VSN RELEASE_COMMAND RELEASE_PROG RELEASE_MODE
                        RELEASE_NODE RELEASE_DISTRIBUTION RELEASE_BOOT_SCRIPT
                        RELEASE_BOOT_SCRIPT_CLEAN)

  @behaviour Plug

  import Plug.Conn

  @impl Plug
  def init(options), do: options

  @impl Plug
  def call(%Plug.Conn{method: "GET", path_info: []} = conn, _options) do
    conn
    |> put_resp_content_type("text/plain")
    |> send_resp(200, "Hello world from PBeam!")
  end

  @impl Plug
  def call(%Plug.Conn{method: "GET", path_info: ["argv"]} = conn, _options) do
    conn
    |> put_resp_content_type("text/plain")
    |> send_resp(200, inspect(System.argv()))
  end

  @impl Plug
  def call(%Plug.Conn{method: "GET", path_info: ["release"]} = conn, _options) do
    conn
    |> put_resp_content_type("text/plain")
    |> send_resp(200, inspect(Enum.map(@release_variables, &System.get_env/1)))
  end

  @impl Plug
  def call(%Plug.Conn{method: "GET", path_info: ["executable"]} = conn, _options) do
    conn
    |> put_resp_content_type("text/plain")
    |> send_resp(200, inspect(System.get_env("PBEAM_EXECUTABLE")))
  end

  def call(conn, _options), do: send_resp(conn, 404, "Not found")
end

defmodule PlugHello.Application do
  @moduledoc "Starts Bandit with `PlugHello` on the port of the `:port` application environment."
  use Application

  @impl Application
  def start(_type, _args) do
    children = [{Bandit, plug: PlugHello, port: Application.fetch_env!(:plug_hello, :port)}]
    Supervisor.start_link(children, strategy: :one_for_one, name: PlugHello.Supervisor)
  end
end
