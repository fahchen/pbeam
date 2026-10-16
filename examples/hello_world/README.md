# Hello World

This example runs `HelloWorld.main/0` of `hello_world.ex` with the `pbeam` binary. The function writes `Hello, world!` with `IO.puts/1`. The `pbeam` binary loads the BEAM files that you give and runs one function as the root process. It does not start the OTP standard IO servers. The boot module `pbeam_boot` of this example starts them.

## Boot module

`pbeam_boot:start(Module, Function, Flags)` does the work of the ERTS boot that `erl -noshell` needs to write to standard output. The root process traps exits and registers an `init` request process with the flags `[{noshell, []} | Flags]`. It needs the root flag, as `init` does (`erts/preloaded/src/init.erl:1226-1232`). It then starts the pinned `erl_prim_loader` with the code path of the `--pa` directories and `"."`, as the `init` boot process does before the boot script (`erts/preloaded/src/init.erl:1164-1184`, `1196-1202`), and sets the path of the pinned `start_clean.boot`. It then runs the other commands of `start_clean.boot` in their order: `logger_server:start_link/0` and `application_controller:start/1` with the kernel application spec, with the link of each process given to `init`, `application:load/1` with the `stdlib` spec, `application:start_boot/2` of `kernel` and `stdlib`, and `c:erlangrc/0` (`erts/preloaded/src/init.erl:1335-1361`). `kernel:start/2` starts `kernel_sup` and its children (`lib/kernel/src/kernel.erl:35-45`, `91-227`), so the boot starts no kernel process by hand. The root process then sets the registered `user` process as its group leader and calls `Module:Function()`, as the `init` boot process runs `-eval` (`erts/preloaded/src/init.erl:1192-1217`, `1539-1546`). See "Boot" in [ERTS Differences](../../docs/erts-differences.md) for the differences from ERTS.

## Run

From the repository root, this command builds the example and runs it with the `pbeam` binary. It prints `Hello, world!` and exits with status 0:

```sh
mise run example hello_world
```

Add `--erts` to run the same entry on the pinned ERTS with `erl +S 1 -noshell`. The `+S 1` flag gives ERTS one scheduler, as the `pbeam` binary has one scheduler thread:

```sh
mise run example hello_world --erts
```

The boot needs `--root` with the root directory of the pinned OTP, as `erl` gives `-root` to `init`. Without it, the run stops with `pbeam: root process exited with no_or_multiple_root_variables` and status 1. Give the BEAM files of the 45 boot modules to the `pbeam` binary. The pinned `error_handler` and `code_server` load each other module of the kernel start on demand from the `kernel` and `stdlib` directories of the root. In a locale whose codeset is not UTF-8, such as `LC_ALL=C`, only ASCII output is supported. Output that is not ASCII stops the run with a runtime error, for example `unsupported bs_create_bin segment integer` for `é`, and status 1 (see "Boot" in [ERTS Differences](../../docs/erts-differences.md)). Run the command in a locale whose codeset is UTF-8, such as `LC_ALL=C.UTF-8`, to write any output.

The task runs the commands that follow, with the `mise` tools on `PATH`. The task also consolidates the protocols of Elixir into `target/examples/hello_world/consolidated/`, as `mix run` does. It gives the `pbeam` binary a start set: the example modules, `pbeam_boot`, the 45 boot modules, the preloaded modules of ERTS that PBeam loads from a BEAM file, and the consolidated protocols that the example calls (`scripts/example_beams.exs`). Each other module of the example loads on demand through the pinned `error_handler` and `code_server`, as in an interactive-mode ERTS run (see "Code loading" in [ERTS Differences](../../docs/erts-differences.md)). The task also gives `--root` with the root directory of the pinned OTP, and `--pa` with each directory of the code path (see the `pbeam` options in the [README](../../README.md)). It writes each called module whose BEAM file the PBeam loader does not load, such as `calendar` (`unsupported BEAM opcode 185`), with the reason into `target/examples/hello_world/pbeam.unloaded`. A call to such a module stops the `pbeam` run with `unresolved external MFA`. Hello World does not call the other modules of the example build, so the commands give only `Elixir.IO`, the boot modules, and `--root`, and do not consolidate the protocols. It does not set `LC_ALL`, so the run uses the locale of the shell. Run them in `sh` or `bash` from the repository root to do the same steps by hand. They compile `hello_world.ex` with the Elixir version and `pbeam_boot.erl` with the OTP version in `mise.toml` into `target/examples/hello_world/`, find the BEAM files of the boot modules and of `Elixir.IO` in the pinned OTP and Elixir, and run `pbeam_boot:start('Elixir.HelloWorld', main)`. The run prints `Hello, world!` and exits with status 0:

```sh
mkdir -p target/examples/hello_world
mise exec -- elixirc --warnings-as-errors -o target/examples/hello_world examples/hello_world/hello_world.ex
mise exec -- erlc +deterministic -o target/examples/hello_world examples/hello_world/pbeam_boot.erl
boot_modules="erlang prim_tty proc_lib application application_controller lists os unicode gen_event gen prim_tty_sighandler erl_signal_handler standard_error file_server supervisor_bridge user_sup user_drv group gen_statem io io_lib io_lib_format proplists queue gen_server logger logger_config edlin edlin_key logger_server logger_simple_h logger_sup supervisor logger_std_h logger_h_common logger_olp logger_proxy logger_handler_watcher logger_formatter maps init erts_internal erl_prim_loader prim_file error_handler"
boot_beams=$(mise exec -- erl -noshell -eval '[io:format("~ts~n", [case code:which(list_to_atom(M)) of preloaded -> filename:join([code:lib_dir(erts), "ebin", M ++ ".beam"]); Path -> Path end]) || M <- init:get_plain_arguments()], halt().' -extra $boot_modules)
elixir_io=$(mise exec -- elixir -e 'IO.puts(:code.which(IO))')
root=$(mise exec -- erl -noshell -eval 'io:put_chars(code:root_dir()), halt().')
LC_ALL=C.UTF-8 cargo run --bin pbeam -- pbeam_boot start Elixir.HelloWorld main --root "$root" -- target/examples/hello_world/Elixir.HelloWorld.beam $elixir_io $boot_beams target/examples/hello_world/pbeam_boot.beam
```

With `--erts`, the task runs the same entry on the pinned ERTS after the compile commands:

```sh
mise exec -- erl +S 1 -noshell -pa target/examples/hello_world -pa "$(dirname "$elixir_io")" -eval "'Elixir.HelloWorld':main(), halt()."
```

The arguments after the module and function name are atoms. The arguments after `--` are the BEAM files to load. Run `cargo run --bin pbeam -- --help` to print the command options.
