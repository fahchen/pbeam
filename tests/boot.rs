use std::{
    env, fs,
    process::{Command, Stdio},
};

#[path = "common/boot.rs"]
mod boot;
#[expect(
    dead_code,
    reason = "this test crate gives BEAM paths, not bytes, to the binary"
)]
mod common;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::pinned_beam_path;

const BOOT_PROBE: &str = "fixtures/boot/boot_probe.beam";
const BOOT_SOURCE: &str = "examples/hello_world/pbeam_boot.erl";

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
