{version, 0} = System.cmd("mise", ~w(config get --file mise.toml tools.rust.version))

{_, status} =
  System.cmd("rustup", ~w(component add rustfmt clippy --toolchain) ++ [String.trim(version)],
    into: IO.stream(:stdio, :line)
  )

System.halt(status)
