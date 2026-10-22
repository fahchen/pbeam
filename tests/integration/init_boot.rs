//! Boot the node with the pinned `init` and a fixed boot script, and compare
//! the result with the pinned ERTS.

use std::{
    env,
    os::unix::{ffi::OsStrExt, process::ExitStatusExt},
    process::{Command, Stdio},
};

use pbeam_runtime::{BeamError, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

/// The boot file of the probe, without its `.boot` extension, as the
/// `-boot` flag of `erl` takes it (init.erl:1256-1267).
const BOOT_FILE: &str = "fixtures/boot/init_boot_probe";
const PROBE: &[u8] = include_bytes!("../../fixtures/boot/init_boot_probe.beam");
const LOADED_ORDER_PROBE: &[u8] = include_bytes!("../../fixtures/boot/loaded_order_probe.beam");
const SYSTEM_PROCESS_PROBE: &[u8] = include_bytes!("../../fixtures/boot/system_process_probe.beam");

/// The environment variable that `boots_init_with_a_fixed_boot_script_as_pinned_erts`
/// sets for its child, so the boot runs in a process of its own: the probe
/// halts the node.
const ISOLATED: &str = "PBEAM_INIT_BOOT_ISOLATED";

/// The pinned modules that the boot runs before the probe: `erl_init` and
/// the modules that `erl_init:start/2` and `init` call
/// (erl_init.erl:35-50; init.erl:586-596, 1164-1358).
const BOOT_MODULES: [&str; 10] = [
    "erl_init",
    "init",
    "erl_prim_loader",
    "prim_file",
    "zlib",
    "erl_tracer",
    "prim_buffer",
    "erlang",
    "erts_internal",
    "lists",
];

/// The `root` and `bindir` flags that `erl` gives the pinned emulator, as
/// `init:get_argument/1` gives them.
fn emulator_flags() -> (Vec<u8>, Vec<u8>) {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "{ok, [[R]]} = init:get_argument(root), {ok, [[B]]} = init:get_argument(bindir), \
             io:put_chars([R, $\\n, B]), halt().",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let (root, bindir) = output.stdout.split_at(
        output
            .stdout
            .iter()
            .position(|&byte| byte == b'\n')
            .unwrap(),
    );
    (root.to_vec(), bindir[1..].to_vec())
}

#[test]
fn boots_init_with_a_fixed_boot_script_as_pinned_erts() {
    // `erl -boot File` runs the boot script of `File.boot` in the boot
    // process of `init` (init.erl:1192-1217, 1256-1308). The probe records
    // the boot status of `init` at each step of the script, waits for the
    // exit of the boot process, and displays the registered name of `init`,
    // the steps, and the status `{started, started}` of the main loop of
    // `init` (init.erl:700-704). It then halts with status 0.
    let reference = Command::new("erl")
        .args(["-noshell", "-boot", BOOT_FILE, "-pa", "fixtures/boot"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let expected = "{boot_exit,normal,init,\"<0.0.0>\",{registered_name,init},\
        status,{started,started},script_id,{\"init_boot_probe\",\"1\"},\
        recorder_linked_to_init,true,\
        steps,[{first,{starting,init_kernel_started}},{second,{starting,applications_loaded}}],\
        names,[{init,true},{erl_prim_loader,true},{init__boot__on_load_handler,true},\
        {init_boot_probe_recorder,true}],alive,true}\r\n";
    assert_eq!(
        (
            String::from_utf8_lossy(&reference.stdout).as_ref(),
            reference.stderr.as_slice(),
            reference.status.code()
        ),
        (expected, &b""[..], Some(0)),
        "ERTS"
    );

    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "isolated_init_boot",
            "--include-ignored",
            "--quiet",
        ])
        .env(ISOLATED, "1")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert!(
        child.status.success(),
        "child exited with {}\n{stdout}",
        child.status
    );
    let lines = stdout
        .split_inclusive('\n')
        .filter(|line| line.starts_with("{boot_exit"))
        .collect::<Vec<_>>();
    assert_eq!(lines, [expected], "PBeam");
}

/// The child side of `boots_init_with_a_fixed_boot_script_as_pinned_erts`:
/// boot the node with the arguments that `erl` gives the emulator, and
/// expect the halt of the probe.
#[test]
#[ignore = "boots_init_with_a_fixed_boot_script_as_pinned_erts runs it in a child process"]
fn isolated_init_boot() {
    assert!(
        env::var_os(ISOLATED).is_some(),
        "run it through its parent test"
    );
    let mut beams = BOOT_MODULES.map(pinned_beam).to_vec();
    beams.push(PROBE.to_vec());
    let slices = beams.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let program = BeamProgram::load(&slices).unwrap();
    let (root, bindir) = emulator_flags();
    let home = env::var_os("HOME").unwrap();
    let halted = program.boot(&[
        b"-root",
        &root,
        b"-bindir",
        &bindir,
        b"-progname",
        b"erl",
        b"--",
        b"-home",
        home.as_bytes(),
        b"--",
        b"-noshell",
        b"-boot",
        BOOT_FILE.as_bytes(),
        b"-pa",
        b"fixtures/boot",
    ]);
    assert_eq!(halted.unwrap_err(), BeamError::Halt(0));
}

/// Return the standard error text of `system_process_probe:Function()` on
/// the pinned ERTS, which writes no crash dump with
/// `ERL_CRASH_DUMP_SECONDS=0`, with its exit status and the signal that
/// stopped it.
fn erts_stop(function: &str) -> (String, Option<i32>, Option<i32>) {
    let output = Command::new("erl")
        .args(["-noshell", "-pa", "fixtures/boot", "-eval"])
        .arg(format!("system_process_probe:{function}()."))
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (
        String::from_utf8(output.stderr).unwrap(),
        output.status.code(),
        output.status.signal(),
    )
}

fn run_system_process_probe(function: &str) -> Result<pbeam_runtime::HostTerm, BeamError> {
    BeamProgram::load(&[SYSTEM_PROCESS_PROBE]).unwrap().execute(
        "system_process_probe",
        function,
        &[],
    )
}

#[test]
fn stops_the_run_at_the_exit_of_a_system_process_as_pinned_erts() {
    // The process of `erts_internal:spawn_system_process/3` is a system
    // process (erl_init.c:333-353, 436-456), and the exit of a system
    // process stops the emulator with the text `System process <Pid>
    // terminated: <Reason>` (erl_process.c:14186-14188), status 1 with no
    // crash dump. The pid of the ERTS process depends on the processes of
    // the boot, so only its form is compared.
    let (stderr, status, _) = erts_stop("exits");
    let text = stderr.lines().next().unwrap();
    let pid = text
        .strip_prefix("System process <0.")
        .and_then(|rest| rest.strip_suffix(".0> terminated: normal"))
        .unwrap_or_else(|| panic!("ERTS: {stderr}"));
    assert!(
        pid.bytes().all(|byte| byte.is_ascii_digit()),
        "ERTS: {stderr}"
    );
    assert_eq!(status, Some(1), "ERTS");
    assert_eq!(
        run_system_process_probe("exits").unwrap_err(),
        BeamError::Runtime("System process <0.1.0> terminated: normal".into())
    );
}

#[test]
fn stops_the_run_for_a_system_process_without_an_export_as_pinned_erts() {
    // `erl_spawn_system_process` looks up the export first and stops the
    // emulator with `No function M:F/A` when it finds none; the arity of an
    // improper list is -1 (erl_init.c:342-346). ERTS aborts with SIGABRT
    // after the text; PBeam stops the run with the text as its error.
    for (function, text) in [
        (
            "absent",
            "No function system_process_probe:absent_function/0",
        ),
        ("improper", "No function system_process_probe:exits/-1"),
    ] {
        let (stderr, _, signal) = erts_stop(function);
        assert_eq!(
            (stderr.lines().next(), signal),
            (Some(text), Some(6)),
            "ERTS {function}"
        );
        assert_eq!(
            run_system_process_probe(function).unwrap_err(),
            BeamError::Runtime(text.into()),
            "PBeam {function}"
        );
    }
}

#[test]
fn lists_the_loaded_modules_with_the_last_loaded_first() {
    // `erlang:loaded/0` gives the modules of the module table, the last
    // index first (beam_bif_load.c:762-791). The module table of ERTS holds
    // the preloaded modules first, so this test compares the order of the
    // PBeam start modules, which load in the order of the program.
    let program = BeamProgram::load(&[PROBE, SYSTEM_PROCESS_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let loaded = program
        .execute("system_process_probe", "loaded", &[])
        .unwrap();
    assert_eq!(
        loaded.term,
        Term::list_from(vec![atom("system_process_probe"), atom("init_boot_probe")])
    );
}

#[test]
fn keeps_the_module_table_order_of_loaded_after_a_delete_and_a_reload_as_pinned_erts() {
    // `erlang:finish_loading/1` puts the module entry at the first load
    // (beam_bif_load.c:425), and the entry keeps its index after
    // `erlang:delete_module/1`, so a module with only old code stays in
    // `erlang:loaded/0` and keeps its place after a later load
    // (beam_bif_load.c:762-791). `loaded_order_probe:main/0` loads
    // `loaded_order_b` before `loaded_order_a`, deletes `loaded_order_b`,
    // and loads it again. It keeps only its own two modules of each list,
    // because the other loaded modules of ERTS and PBeam differ: ERTS has
    // its preloaded and boot modules, and PBeam has the start modules.
    let expected = "{[loaded_order_a,loaded_order_b],[loaded_order_a,loaded_order_b],\
        [loaded_order_a,loaded_order_b]}\n";
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/boot",
            "-eval",
            "io:format(\"~w~n\", [loaded_order_probe:main()]), halt().",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        (
            String::from_utf8_lossy(&reference.stdout).as_ref(),
            reference.status.code()
        ),
        (expected, Some(0)),
        "ERTS"
    );
    let erlang = pinned_beam("erlang");
    let prim_file = pinned_beam("prim_file");
    let program = BeamProgram::load(&[&erlang, &prim_file, LOADED_ORDER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let order = || Term::list_from(vec![atom("loaded_order_a"), atom("loaded_order_b")]);
    let result = program.execute("loaded_order_probe", "main", &[]).unwrap();
    assert_eq!(
        result.term,
        Term::tuple(vec![order(), order(), order()]),
        "PBeam"
    );
}
