use std::{
    env, fs,
    process::{Command, Stdio},
};

use pbeam_runtime::{BeamProgram, Map, Term};

#[path = "common/boot.rs"]
mod boot;
mod common;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::{pinned_beam, pinned_beam_path};

const BOOT_PROBE: &str = "fixtures/boot/boot_probe.beam";
const LOGGER_BOOT_PROBE: &[u8] = include_bytes!("../fixtures/boot/logger_boot_probe.beam");
const INIT_STOP_PROBE: &[u8] = include_bytes!("../fixtures/boot/init_stop_probe.beam");
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
        .arg(BOOT_BEAM);
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
    let registered = |name: &str, value: &str| Term::Tuple(vec![atom(name), atom(value)]);
    let expected = Term::Tuple(vec![
        Term::List(vec![atom("default")]),
        atom("notice"),
        Term::Map(
            Map::from_pairs(vec![
                (atom("id"), atom("default")),
                (atom("module"), atom("logger_std_h")),
                (atom("level"), atom("all")),
            ])
            .unwrap(),
        ),
        Term::List(vec![
            registered("application_controller", "true"),
            registered("logger", "true"),
            registered("logger_sup", "true"),
            registered("logger_proxy", "true"),
            registered("logger_handler_watcher", "true"),
            registered("logger_std_h_default", "true"),
            registered("logger_simple_h", "false"),
        ]),
        Term::Tuple(vec![atom("ok"), atom("notice")]),
    ]);

    assert_eq!(
        program
            .execute(
                "pbeam_boot",
                "start",
                &[atom("logger_boot_probe"), atom("main")]
            )
            .unwrap(),
        expected
    );
}

#[test]
fn keeps_the_init_stop_request_in_the_mailbox_of_the_init_request_process() {
    // `init:stop/0` sends `{stop, stop}` to the registered `init` process and
    // returns `ok` (init.erl:536). The `init` request process of
    // `pbeam_boot` keeps this message in its mailbox and does not exit, so
    // the node does not stop (see "Run end" in docs/erts-differences.md).
    // `isolated_init_stop` does the PBeam run.
    run_isolated("isolated_init_stop");
}

/// The child side of
/// `keeps_the_init_stop_request_in_the_mailbox_of_the_init_request_process`:
/// run the boot and `init_stop_probe:main/0`, which gives `running` when
/// `init` does not exit.
#[test]
#[ignore = "keeps_the_init_stop_request_in_the_mailbox_of_the_init_request_process runs it in a child process"]
fn isolated_init_stop() {
    let program = boot_program(INIT_STOP_PROBE);
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());

    assert_eq!(
        program
            .execute(
                "pbeam_boot",
                "start",
                &[atom("init_stop_probe"), atom("main")]
            )
            .unwrap(),
        atom("running")
    );
}

/// Load the pinned boot modules, `probe`, and the boot module.
fn boot_program(probe: &[u8]) -> BeamProgram {
    assert!(
        env::var_os(ISOLATED).is_some(),
        "run_isolated runs this test in a child process"
    );
    let beams = BOOT_MODULES.map(pinned_beam);
    let boot = fs::read(BOOT_BEAM).unwrap();
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
    // `logger_server:start_link/0` first, as start_clean.boot starts the
    // kernel process `logger` first after `heart` (init.erl:1348-1350), and
    // the run stops there.
    let output = Command::new(env!("CARGO_BIN_EXE_pbeam"))
        .args([
            "pbeam_boot",
            "start",
            "boot_probe",
            "main",
            "--",
            BOOT_PROBE,
            BOOT_BEAM,
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
        (
            &b""[..],
            &b"pbeam: unresolved external MFA logger_server:start_link/0\n"[..],
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

#[test]
fn checks_in_the_deterministic_build_of_the_boot_module_source() {
    let directory = env::temp_dir().join(format!("pbeam-boot-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let status = Command::new("erlc")
        .args(["+deterministic", "-o"])
        .arg(&directory)
        .arg(BOOT_SOURCE)
        .status()
        .unwrap();
    let compiled = fs::read(directory.join("pbeam_boot.beam")).unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert!(status.success());
    assert_eq!(compiled, fs::read(BOOT_BEAM).unwrap());
}
