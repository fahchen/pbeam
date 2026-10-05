use std::{
    env, fs,
    process::{Command, Stdio},
};

#[path = "common/boot.rs"]
mod boot;
mod common;

use boot::BOOT_MODULES;
use common::{pinned_beam, pinned_beam_path};
use pbeam_runtime::BeamProgram;

const BOOT_PROBE: &str = "fixtures/boot/boot_probe.beam";
const BOOT_SOURCE: &str = "src/runtime/interpreter/boot/pbeam_boot.erl";
const BOOT_BEAM: &str = "src/runtime/interpreter/boot/pbeam_boot.beam";

#[test]
fn boots_the_standard_io_servers_before_the_entry_of_the_pbeam_binary() {
    // `boot_probe:main/0` matches `whereis(user)` with `group_leader()` and
    // `true` with `process_flag(trap_exit, true)`, as the `-eval` process of
    // `erl -noshell` has `user` as its group leader and traps exits
    // (init.erl:1197, 1539-1546). It then writes "Hello, world!\n" with
    // `io:put_chars/2` through `user`, which `user_sup` starts with
    // `user_drv:start(#{initial_shell => noshell, input => cooked})`
    // (user_sup.erl:129-130). A failed match exits with status 1.
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
    pbeam.args(["boot_probe", "main", BOOT_PROBE]).args(&beams);
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
fn loads_the_boot_module_after_the_given_modules() {
    // The boot module loads last, so each atom of the given modules has the
    // index that `BeamProgram::load` gives it.
    let probe = fs::read(BOOT_PROBE).unwrap();
    let erlang = pinned_beam("erlang");
    let loaded = BeamProgram::load(&[&probe, &erlang]).unwrap();
    let booted = BeamProgram::load_with_boot(&[&probe, &erlang]).unwrap();

    for name in ["boot_probe", "main", "put_chars", "nif_error"] {
        assert_eq!(booted.atom(name), loaded.atom(name), "{name}");
        assert!(booted.atom(name).is_some(), "{name}");
    }
    assert_eq!(loaded.atom("pbeam_boot"), None);
    assert!(booted.atom("pbeam_boot").is_some());

    let boot = fs::read(BOOT_BEAM).unwrap();
    let error = BeamProgram::load_with_boot(&[&boot]).unwrap_err();
    assert_eq!(error.to_string(), "duplicate BEAM module pbeam_boot");
}

#[test]
fn checks_the_entry_export_before_the_boot() {
    // The program has no boot modules. The entry checks give the errors of
    // `BeamProgram::execute` before the boot runs, and the boot then stops
    // at the first missing module.
    let probe = fs::read(BOOT_PROBE).unwrap();
    let program = BeamProgram::load_with_boot(&[&probe]).unwrap();

    let errors =
        [("missing_probe", "main"), ("boot_probe", "missing")].map(|(module, function)| {
            program
                .execute_after_boot(module, function)
                .unwrap_err()
                .to_string()
        });
    assert_eq!(
        errors,
        [
            "missing BEAM module missing_probe",
            "missing export boot_probe:missing/0"
        ]
    );
    let error = program
        .execute_after_boot("boot_probe", "main")
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "unresolved external MFA standard_error:start_link/0"
    );
}

#[test]
fn embeds_the_deterministic_build_of_the_boot_module_source() {
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
