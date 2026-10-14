use std::{
    env, fs,
    process::{Command, Stdio},
};

use pbeam_runtime::{BeamError, BeamProgram, Map, Term};

#[path = "../common/boot.rs"]
mod boot;
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam};
use common::{pinned_beam, pinned_beam_path};

const BOOT_PROBE: &str = "fixtures/boot/boot_probe.beam";
const LOGGER_BOOT_PROBE: &[u8] = include_bytes!("../../fixtures/boot/logger_boot_probe.beam");
const INIT_STOP_PROBE: &[u8] = include_bytes!("../../fixtures/boot/init_stop_probe.beam");
const BOOT_SOURCE: &str = "examples/hello_world/pbeam_boot.erl";

/// The environment variable that `run_isolated` sets for its child, so an
/// isolated test does not run the boot in the process of the other tests.
const ISOLATED: &str = "PBEAM_BOOT_ISOLATED";

#[test]
fn boots_the_standard_io_servers_before_the_entry_of_the_pbeam_binary() {
    // `boot_probe:main/0` matches `whereis(user)` with `group_leader()` and
    // `true` with `process_flag(trap_exit, true)`, as the `-eval` process of
    // `erl -noshell` has `user` as its group leader and traps exits
    // (init.erl:1197, 1539-1546). It then writes "Hello, world!\n" with
    // `io:put_chars/2` through `user`, which `user_sup` starts with
    // `user_drv:start(#{initial_shell => noshell, input => cooked})`
    // (user_sup.erl:129-130). A failed match exits with status 1. The
    // `pbeam` binary runs `pbeam_boot:start(boot_probe, main)`, which starts
    // these servers first.
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let mut erts = Command::new("erl");
    erts.args([
        "-noshell",
        "-pa",
        "fixtures/boot",
        "-eval",
        "boot_probe:main(), halt().",
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args([
            "pbeam_boot",
            "start",
            "boot_probe",
            "main",
            "--",
            BOOT_PROBE,
        ])
        .args(&beams)
        .arg(boot_beam());
    for (name, command) in [("ERTS", &mut erts), ("PBeam", &mut pbeam)] {
        let output = command
            .env("TERM", "dumb")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            (
                output.stdout.as_slice(),
                output.stderr.as_slice(),
                output.status.code()
            ),
            (&b"Hello, world!\n"[..], &b""[..], Some(0)),
            "{name}"
        );
    }
}

#[test]
fn starts_the_logger_and_the_application_controller_as_pinned_erts() {
    // `pbeam_boot:start/2` starts `logger_server` and
    // `application_controller` as kernel processes through its `init`
    // request process (init.erl:1519-1528, 768-772). Its kernel process then
    // runs `logger:internal_init_logger/0`, the `kernel_sup` children, and
    // `logger:add_handlers(kernel)` (kernel.erl:37-42). `add_handler/3`
    // removes the `simple` handler after it adds `default`
    // (logger.erl:838-848), and the kernel spec gives the primary level
    // `notice` (logger.erl:1748-1754). `logger_boot_probe:main/0` reads this
    // state after the boot. `isolated_logger_boot` does the PBeam run.
    run_isolated("isolated_logger_boot");

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/boot",
            "-eval",
            "io:format(\"~w~n\", [logger_boot_probe:main()]), halt().",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        (
            reference.stdout.as_slice(),
            reference.stderr.as_slice(),
            reference.status.code()
        ),
        (
            &b"{[default],notice,#{id => default,module => logger_std_h,level => all},\
               [{application_controller,true},{logger,true},{logger_sup,true},\
               {logger_proxy,true},{logger_handler_watcher,true},\
               {logger_std_h_default,true},{logger_simple_h,false}],{ok,notice}}\n"[..],
            &b""[..],
            Some(0)
        )
    );
}

/// The child side of
/// `starts_the_logger_and_the_application_controller_as_pinned_erts`: run
/// the boot and `logger_boot_probe:main/0`, and compare the result with the
/// pinned ERTS result.
#[test]
#[ignore = "starts_the_logger_and_the_application_controller_as_pinned_erts runs it in a child process"]
fn isolated_logger_boot() {
    let program = boot_program(LOGGER_BOOT_PROBE);
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let registered = |name: &str, value: &str| Term::tuple(vec![atom(name), atom(value)]);
    let expected = Term::tuple(vec![
        Term::list_from(vec![atom("default")]),
        atom("notice"),
        Term::Map(
            Map::from_pairs(
                vec![
                    (atom("id"), atom("default")),
                    (atom("module"), atom("logger_std_h")),
                    (atom("level"), atom("all")),
                ],
                program.atoms(),
            )
            .unwrap(),
        ),
        Term::list_from(vec![
            registered("application_controller", "true"),
            registered("logger", "true"),
            registered("logger_sup", "true"),
            registered("logger_proxy", "true"),
            registered("logger_handler_watcher", "true"),
            registered("logger_std_h_default", "true"),
            registered("logger_simple_h", "false"),
        ]),
        Term::tuple(vec![atom("ok"), atom("notice")]),
    ]);

    assert_eq!(
        program
            .execute(
                "pbeam_boot",
                "start",
                &[
                    program.host_term(atom("logger_boot_probe")),
                    program.host_term(atom("main")),
                ]
            )
            .unwrap()
            .term,
        expected
    );
}

#[test]
fn halts_at_init_stop_as_pinned_erts() {
    // `init:stop/0` sends `{stop, stop}` to the registered `init` process and
    // returns `ok` (init.erl:536). `init` shuts down the kernel processes and
    // calls `halt()` (init.erl:924-945), so the probe never writes its
    // result, and the node exits with status 0. The `init` request process
    // of `pbeam_boot` stops its kernel process and calls `halt()`.
    // `isolated_init_stop` does the PBeam run.
    run_isolated("isolated_init_stop");

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/boot",
            "-eval",
            "io:format(\"~w~n\", [init_stop_probe:main()]), halt(3).",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        (
            reference.stdout.as_slice(),
            reference.stderr.as_slice(),
            reference.status.code()
        ),
        (&b""[..], &b""[..], Some(0))
    );
}

/// The child side of `halts_at_init_stop_as_pinned_erts`: run the boot and
/// `init_stop_probe:main/0`, which halts the run with status 0.
#[test]
#[ignore = "halts_at_init_stop_as_pinned_erts runs it in a child process"]
fn isolated_init_stop() {
    let program = boot_program(INIT_STOP_PROBE);
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());

    assert_eq!(
        program.execute(
            "pbeam_boot",
            "start",
            &[
                program.host_term(atom("init_stop_probe")),
                program.host_term(atom("main")),
            ]
        ),
        Err(BeamError::Halt(0))
    );
}

/// Load the pinned boot modules, `probe`, and the boot module.
fn boot_program(probe: &[u8]) -> BeamProgram {
    assert!(
        env::var_os(ISOLATED).is_some(),
        "run_isolated runs this test in a child process"
    );
    let beams = BOOT_MODULES.map(pinned_beam);
    let boot = fs::read(boot_beam()).unwrap();
    let mut modules: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    modules.extend([probe, boot.as_slice()]);
    BeamProgram::load(&modules).unwrap()
}

/// Run the ignored test `name` of this test binary in a child process, and
/// require that it passes. A child process is necessary, because the boot
/// starts `prim_tty` in the `cooked` input mode, and the
/// `prim_tty_sighandler` init calls `os:set_signal/2` for `sigcont` and
/// `sigwinch` (prim_tty.erl:284-298, prim_tty_sighandler.erl:28-31). A
/// disposition is the same for all threads of a process, so it would change
/// the process of the other tests. The child inherits `LLVM_PROFILE_FILE`,
/// so `cargo llvm-cov` measures it. `LC_ALL=C.UTF-8` gives the child a UTF-8
/// codeset, as the other boot tests have, also in a shell with no locale
/// variable.
fn run_isolated(name: &str) {
    let child = Command::new(env::current_exe().unwrap())
        .args(["--exact", name, "--include-ignored", "--quiet"])
        .env(ISOLATED, "1")
        .env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{name} exited with {}\n{}",
        child.status,
        String::from_utf8_lossy(&child.stdout)
    );
}

#[test]
fn stops_the_boot_at_the_first_missing_otp_module() {
    // Without the pinned OTP modules, the boot process calls
    // `erl_prim_loader:start/0` first, as the `init` boot process starts the
    // loader before it runs start_clean.boot (init.erl:1196-1202), and the
    // run stops there. The boot registers the `init` process and starts it,
    // so the export entry of the call runs the default error handler
    // (`call_error_handler`, `beam_common.c:1294-1342`). The run does not
    // load `error_handler` and no code server answers a load, so
    // `error_handler:undefined_function/3` is not found and the call raises
    // `error:undef` with the frame of its initial call.
    let output = Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args([
            "pbeam_boot",
            "start",
            "boot_probe",
            "main",
            "--",
            BOOT_PROBE,
        ])
        .arg(boot_beam())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(
        (
            output.stdout.as_slice(),
            output.stderr.as_slice(),
            output.status.code()
        ),
        (
            &b""[..],
            &b"pbeam: root process exited with {undef,[{erl_prim_loader,start,[],[]}]}\n"[..],
            Some(1)
        )
    );
}

#[test]
fn keeps_the_kernel_spec_of_the_pinned_start_clean_boot() {
    // `pbeam_boot:start/2` gives `kernel_app/0` to
    // `application_controller:start/1`, as start_clean.boot gives its kernel
    // application spec (init.erl:1348-1350). The pinned ERTS parses the
    // boot module source and compares the body of `kernel_app/0` with the
    // spec in its start_clean.boot.
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "{ok, Boot} = file:read_file(filename:join([code:root_dir(), \"bin\", \"start_clean.boot\"])), \
             {script, _, Commands} = binary_to_term(Boot), \
             [Spec] = [App || {kernelProcess, application_controller, {_, _, [App]}} <- Commands], \
             [Source] = init:get_plain_arguments(), \
             {ok, Forms} = epp:parse_file(Source, []), \
             [[{clause, _, [], [], [Expr]}]] = [Clauses || {function, _, kernel_app, 0, Clauses} <- Forms], \
             io:format(\"~p~n\", [erl_parse:normalise(Expr) =:= Spec]), \
             halt().",
            "-extra",
            BOOT_SOURCE,
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(
        (
            output.stdout.as_slice(),
            output.stderr.as_slice(),
            output.status.code()
        ),
        (&b"true\n"[..], &b""[..], Some(0))
    );
}
