use std::{
    env, fs,
    os::unix::ffi::OsStrExt,
    process::{Command, Stdio},
};

use pbeam_runtime::{BeamError, BeamProgram, Map, Term};

#[path = "../common/boot.rs"]
mod boot;
#[path = "../common/mod.rs"]
mod common;

use boot::{BOOT_MODULES, boot_beam, otp_root};
use common::{pinned_beam, pinned_beam_path};

const BOOT_PROBE: &str = "fixtures/boot/boot_probe.beam";
const KERNEL_APP_PROBE: &str = "fixtures/boot/kernel_app_probe.beam";
const KERNEL_EXIT_PROBE: &str = "fixtures/boot/kernel_exit_probe.beam";
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
            "--root",
            otp_root().to_str().unwrap(),
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
    // request process (init.erl:1519-1528, 768-772).
    // `application:start_boot(kernel, permanent)` then runs `kernel:start/2`,
    // which runs `logger:internal_init_logger/0`, the `kernel_sup` children,
    // and `logger:add_handlers(kernel)` (kernel.erl:35-45). `add_handler/3`
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
                    program.host_term(root_flags(&program)),
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
    // of `pbeam_boot` shuts down its kernel processes as `init` does, so
    // `application_controller` stops `kernel` and `stdlib`, and calls
    // `halt()` (init.erl:955-1062).
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
                program.host_term(root_flags(&program)),
            ]
        ),
        Err(BeamError::Halt(0))
    );
}

/// Return the boot flags `[{root, [Root]}]` of `pbeam_boot:start/3`, as the
/// `pbeam` binary gives them for `--root` (row "Boot" of
/// `docs/erts-differences.md`).
fn root_flags(program: &BeamProgram) -> Term {
    let root = Term::binary(otp_root().as_os_str().as_bytes().to_vec());
    Term::list_from(vec![Term::tuple(vec![
        Term::Atom(program.atom("root").unwrap()),
        Term::list_from(vec![root]),
    ])])
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
fn stops_the_boot_without_a_root_flag_as_init_does() {
    // `pbeam_boot:start/3` reads the root flag first, as the `init` boot
    // process does (init.erl:1196-1198). Without `--root`, the `pbeam`
    // binary runs `pbeam_boot:start/2` with no flags, and the boot process
    // exits with `no_or_multiple_root_variables` (init.erl:1226-1232)
    // before it calls a pinned OTP module.
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
            &b"pbeam: root process exited with no_or_multiple_root_variables\n"[..],
            Some(1)
        )
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
            "--root",
            otp_root().to_str().unwrap(),
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
fn starts_the_kernel_application_as_pinned_erts() {
    // `pbeam_boot` runs the commands of `start_clean.boot` after its
    // kernel processes: `application:load/1` of the `stdlib` spec, and
    // `application:start_boot/2` of `kernel` and `stdlib`
    // (init.erl:1335-1361). `kernel:start/2` starts `kernel_sup`, and
    // `kernel:init/1` gives its children (kernel.erl:35-45, 91-227).
    // `kernel_app_probe:main/0` writes `application:which_applications/0`
    // and the id, type, and modules of each `kernel_sup` child. The ERTS
    // run gives `-boot start_clean`, because the default boot of the pinned
    // install also starts `sasl`.
    let expected = "{[{stdlib,[69,82,84,83,32,32,67,88,67,32,49,51,56,32,49,48],[56,46,49]},\
                    {kernel,[69,82,84,83,32,32,67,88,67,32,49,51,56,32,49,48],[49,49,46,48,46,52]}],\
                    [{kernel_safe_sup,supervisor,[kernel]},{kernel_refc,worker,[kernel_refc]},\
                    {kernel_config,worker,[kernel_config]},{logger_sup,supervisor,[logger_sup]},\
                    {user,supervisor,[user_sup]},{erl_signal_server,worker,dynamic},\
                    {global_group,worker,[global_group]},{net_sup,supervisor,[erl_distribution]},\
                    {global_name_server,worker,[global]},{rex,worker,[rpc]},\
                    {inet_db,worker,[inet_db]},{on_load,worker,[kernel]},\
                    {file_server_2,worker,[file,file_server,file_io_server,prim_file]},\
                    {standard_error,supervisor,[standard_error]},{code_server,worker,[code]}]}\n";
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/boot",
        "-eval",
        "kernel_app_probe:main(), halt().",
    ]);
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "kernel_app_probe", "main", "--root"])
        .arg(otp_root())
        .args(["--", KERNEL_APP_PROBE])
        .args(BOOT_MODULES.map(pinned_beam_path))
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
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                output.status.code()
            ),
            (expected.into(), "".into(), Some(0)),
            "{name}"
        );
    }
}

#[test]
fn halts_when_a_kernel_process_exits_as_init_does() {
    // `kernel_exit_probe:main/0` kills `erl_prim_loader`, a kernel process
    // of `init`. `init` waits 500 ms and halts with the text
    // `Kernel pid terminated (Name) (Reason)`, which
    // `erts_internal:term_to_string/2` makes (init.erl:665-679, 783-787,
    // 1139-1148). PBeam does not run that BIF, so the run stops there with
    // status 1 (row "Boot" of docs/erts-differences.md). The ERTS run
    // writes its crash dump to `/dev/null`.
    let mut erts = Command::new("erl");
    erts.args([
        "+S",
        "1",
        "-noshell",
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/boot",
        "-eval",
        "kernel_exit_probe:main().",
    ])
    .env("ERL_CRASH_DUMP", "/dev/null");
    let mut pbeam = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    pbeam
        .args(["pbeam_boot", "start", "kernel_exit_probe", "main", "--root"])
        .arg(otp_root())
        .args(["--", KERNEL_EXIT_PROBE])
        .args(BOOT_MODULES.map(pinned_beam_path))
        .arg(boot_beam());
    let expected = [
        (
            "ERTS",
            &mut erts,
            "Kernel pid terminated (erl_prim_loader) (killed)\r\n\
             \r\nCrash dump is being written to: /dev/null...done\r\n",
        ),
        (
            "PBeam",
            &mut pbeam,
            "pbeam: unresolved external MFA erts_internal:term_to_string/2\n",
        ),
    ];
    for (name, command, stderr) in expected {
        let output = command
            .env("TERM", "dumb")
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                output.status.code()
            ),
            ("".into(), stderr.into(), Some(1)),
            "{name}"
        );
    }
}

#[test]
fn keeps_the_application_specs_of_the_pinned_start_clean_boot() {
    // `pbeam_boot:start/3` gives `kernel_app/0` to
    // `application_controller:start/1` and `stdlib_app/0` to
    // `application:load/1`, as start_clean.boot gives its application specs
    // (init.erl:1348-1361). The pinned ERTS parses the boot module source
    // and compares the bodies of `kernel_app/0` and `stdlib_app/0` with the
    // specs in its start_clean.boot.
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-eval",
            "{ok, Boot} = file:read_file(filename:join([code:root_dir(), \"bin\", \"start_clean.boot\"])), \
             {script, _, Commands} = binary_to_term(Boot), \
             [Kernel] = [App || {kernelProcess, application_controller, {_, _, [App]}} <- Commands], \
             [Stdlib] = [App || {apply, {application, load, [App]}} <- Commands], \
             [Source] = init:get_plain_arguments(), \
             {ok, Forms} = epp:parse_file(Source, []), \
             Body = fun(Name) -> [[{clause, _, [], [], [Expr]}]] = [Clauses || {function, _, F, 0, Clauses} <- Forms, F =:= Name], erl_parse:normalise(Expr) end, \
             io:format(\"~p~n\", [{Body(kernel_app) =:= Kernel, Body(stdlib_app) =:= Stdlib}]), \
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
        (&b"{true,true}\n"[..], &b""[..], Some(0))
    );
}
