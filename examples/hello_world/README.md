# Hello World

This example runs the Elixir Hello World fixture with the `pbeam` binary. The `pbeam` binary loads the BEAM files that you give and runs one function as the root process. It does not start the OTP standard IO servers. The boot module `pbeam_boot` of this example starts them.

## Boot module

`pbeam_boot:start(Module, Function)` does the work of the ERTS boot that `erl -noshell` needs to write to standard output. The root process traps exits and registers an `init` request process with the flags `[{noshell, []}]`. It then runs `logger_server:start_link/0` and `application_controller:start/1` with the kernel application spec of the pinned `start_clean.boot`, and gives the link of each process to `init`, as the `init` boot process does (`erts/preloaded/src/init.erl:1348-1350`, `1519-1528`, `768-772`). The root process then spawns a kernel process. The kernel process runs `logger:internal_init_logger/0`, starts `standard_error`, `erl_signal_server`, `user_sup`, and `logger_sup` in this order, as `kernel_sup` does (`lib/kernel/src/kernel.erl:124-168`, `224-225`), and then runs `erl_signal_handler:start/0` and `logger:add_handlers(kernel)` (`kernel.erl:37-42`). It does not run `os:internal_init_cmd_shell/0` (`kernel.erl:38`), which sets the shell of `os:cmd/1,2`. The root process then sets the registered `user` process as its group leader and calls `Module:Function()`, as the `init` boot process runs `-eval` (`erts/preloaded/src/init.erl:1192-1217`, `1539-1546`). See "Boot" in [ERTS Differences](../../docs/erts-differences.md) for the differences from ERTS.

`pbeam_boot.beam` is the output of this command with the OTP version in `mise.toml`. Run it from the repository root after you change `pbeam_boot.erl`:

```sh
mise exec -- erlc +deterministic -o examples/hello_world examples/hello_world/pbeam_boot.erl
```

## Run

The boot calls 41 pinned OTP modules. Give their BEAM files to the `pbeam` binary. Without one of them, the run stops with `unresolved external MFA` of that module and status 1. In a locale whose codeset is not UTF-8, such as `LC_ALL=C`, only ASCII output is supported. Output that is not ASCII stops the run with `unsupported BEAM opcode 177` and status 1 (see "Boot" in [ERTS Differences](../../docs/erts-differences.md)). Run the command in a locale whose codeset is UTF-8, such as `LC_ALL=C.UTF-8`, to write any output.

From the repository root, these commands compile the Elixir fixture, find the BEAM files of the boot modules and of `Elixir.IO` in the pinned OTP and Elixir, and run `pbeam_boot:start('Elixir.Hello', main)`. The run prints `Hello, world!` and exits with status 0:

```sh
(cd fixtures/hello && mise exec -- mix compile --warnings-as-errors)
boot_modules="erlang prim_tty proc_lib application application_controller lists os unicode gen_event gen prim_tty_sighandler erl_signal_handler standard_error supervisor_bridge user_sup user_drv group gen_statem io io_lib io_lib_format proplists queue gen_server logger logger_config edlin edlin_key logger_server logger_simple_h logger_sup supervisor logger_std_h logger_h_common logger_olp logger_proxy logger_handler_watcher logger_formatter maps init erts_internal"
boot_beams=$(mise exec -- erl -noshell -eval '[io:format("~ts~n", [case code:which(list_to_atom(M)) of preloaded -> filename:join([code:lib_dir(erts), "ebin", M ++ ".beam"]); Path -> Path end]) || M <- init:get_plain_arguments()], halt().' -extra $boot_modules)
elixir_io=$(mise exec -- elixir -e 'IO.puts(:code.which(IO))')
LC_ALL=C.UTF-8 cargo run --bin pbeam -- pbeam_boot start Elixir.Hello main -- fixtures/hello/_build/dev/lib/hello_reference/ebin/Elixir.Hello.beam $elixir_io $boot_beams examples/hello_world/pbeam_boot.beam
```

The arguments after the module and function name are atoms. The arguments after `--` are the BEAM files to load. Run `cargo run --bin pbeam -- --help` to print the command options.
