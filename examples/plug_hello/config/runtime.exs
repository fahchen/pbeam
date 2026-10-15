import Config

# The test run listens on a free port, so that it does not need the port 4000.
port = if config_env() == :test, do: "0", else: System.get_env("PORT", "4000")
config :plug_hello, port: String.to_integer(port)
