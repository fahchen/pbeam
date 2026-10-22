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
/// The boot file of the probe that kills `init`.
const KILL_BOOT_FILE: &str = "fixtures/boot/init_kill_probe";
const KILL_PROBE: &[u8] = include_bytes!("../../fixtures/boot/init_kill_probe.beam");
const LOADED_ORDER_PROBE: &[u8] = include_bytes!("../../fixtures/boot/loaded_order_probe.beam");
const SYSTEM_PROCESS_PROBE: &[u8] = include_bytes!("../../fixtures/boot/system_process_probe.beam");

/// The environment variable that `boots_init_with_a_fixed_boot_script_as_pinned_erts`
/// and the kernel boot tests set for their child, so the boot runs in a
/// process of its own: the probe halts the node, and the kernel boot
/// starts `user_drv`, which sets signal dispositions of the process
/// (prim_tty.erl:284-298). For the kernel boot, the value is the `-s`
/// arguments of the probe.
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
    let halted = boot_probe(PROBE, BOOT_FILE);
    assert_eq!(halted.unwrap_err(), BeamError::Halt(0));
}

/// Boot the node with `probe` and the boot modules, with the arguments that
/// `erl -noshell -boot BootFile -pa fixtures/boot` gives the emulator.
fn boot_probe(probe: &[u8], boot_file: &str) -> Result<pbeam_runtime::HostTerm, BeamError> {
    boot_node(
        &[probe],
        &[b"-boot", boot_file.as_bytes(), b"-pa", b"fixtures/boot"],
    )
}

/// Boot the node with the boot modules and `probes`, with the arguments
/// that `erl -noshell Flags` gives the emulator.
fn boot_node(probes: &[&[u8]], flags: &[&[u8]]) -> Result<pbeam_runtime::HostTerm, BeamError> {
    let beams = BOOT_MODULES.map(pinned_beam);
    let mut slices = beams.iter().map(Vec::as_slice).collect::<Vec<_>>();
    slices.extend(probes);
    let program = BeamProgram::load(&slices).unwrap();
    let (root, bindir) = emulator_flags();
    let home = env::var_os("HOME").unwrap();
    let mut arguments: Vec<&[u8]> = vec![
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
    ];
    arguments.extend(flags);
    program.boot(&arguments)
}

/// The environment of the kernel boot: a home directory and a user
/// configuration directory that do not exist, so `c:erlangrc/0` finds no
/// `.erlang` file of the host user (c.erl:1051-1072).
const KERNEL_BOOT_ENVIRONMENT: [(&str, &str); 2] = [
    ("HOME", "fixtures/boot/no_home"),
    ("XDG_CONFIG_HOME", "fixtures/boot/no_home"),
];

/// The inetrc file `fixtures/boot/kernel_boot.inetrc`, whose domain keeps
/// `inet_config:init/0` from the host name lookup on Linux
/// (inet_config.erl:63-64, 110-133), so the output of the boot does not
/// depend on the host name.
const KERNEL_BOOT_INETRC: (&str, &str) = ("ERL_INETRC", "fixtures/boot/kernel_boot.inetrc");

/// The flags of `erl` for the kernel boot of `kernel_boot_probe`, with the
/// `-s` arguments `arguments`: the function of the probe and its
/// arguments, for example `main 3`.
fn kernel_boot_flags(arguments: &str) -> Vec<String> {
    let mut flags = [
        "-boot",
        "start_clean",
        "-pa",
        "fixtures/boot",
        "-s",
        "kernel_boot_probe",
    ]
    .map(String::from)
    .to_vec();
    flags.extend(arguments.split_whitespace().map(String::from));
    flags.extend(["-kernel", "logger_level", "warning"].map(String::from));
    flags
}

/// Run the kernel boot of `kernel_boot_probe` with the `-s` arguments
/// `arguments` (see [`kernel_boot_flags`]), the environment of the kernel
/// boot, and `ERL_INETRC` only when `inetrc` is true, on the pinned ERTS
/// and on PBeam in a child process. Check that ERTS writes nothing to
/// stderr and that the child passes. Return the stdout and the exit code of
/// ERTS, and the lines of the child that start with one of `prefixes`: the
/// test harness writes its own lines around the output of the node.
fn run_kernel_boot(
    arguments: &str,
    inetrc: bool,
    prefixes: &[&str],
) -> (String, Option<i32>, String) {
    let environment = |command: &mut Command| {
        command
            .envs(KERNEL_BOOT_ENVIRONMENT)
            .env_remove(KERNEL_BOOT_INETRC.0)
            .stdin(Stdio::null());
        if inetrc {
            command.env(KERNEL_BOOT_INETRC.0, KERNEL_BOOT_INETRC.1);
        }
    };
    let mut reference = Command::new("erl");
    reference
        .args(["-noshell"])
        .args(kernel_boot_flags(arguments));
    environment(&mut reference);
    let reference = reference.output().unwrap();
    let mut child = Command::new(env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "isolated_kernel_boot",
            "--include-ignored",
            "--quiet",
        ])
        .env(ISOLATED, arguments);
    environment(&mut child);
    let child = child.output().unwrap();
    let stdout = String::from_utf8_lossy(&child.stdout);
    assert_eq!(
        String::from_utf8_lossy(&reference.stderr),
        "",
        "ERTS with {arguments:?}"
    );
    assert!(
        child.status.success(),
        "child exited with {}\n{stdout}",
        child.status
    );
    let node = stdout
        .split_inclusive('\n')
        .filter(|line| prefixes.iter().any(|prefix| line.starts_with(prefix)))
        .collect();
    (
        String::from_utf8_lossy(&reference.stdout).into_owned(),
        reference.status.code(),
        node,
    )
}

#[test]
fn starts_kernel_and_stops_at_init_stop_as_pinned_erts() {
    // `-boot start_clean` runs the pinned `start_clean.boot` of
    // `$ROOT/bin` (init.erl:1256-1308). Its `primLoad` commands load the
    // kernel and stdlib modules through `erl_prim_loader`
    // (init.erl:1342, 1360-1408), its `kernelProcess` commands start
    // `heart`, `logger`, and `application_controller` as linked kernel
    // processes of `init` (init.erl:1519-1528), and its `apply` commands
    // start `kernel` and `stdlib` with `application:start_boot/2`.
    // `kernel_boot_probe:main/0,1` displays the results of the
    // application API and calls `init:stop/0,1`. `init` shuts down
    // `application_controller`, which stops each application
    // (init.erl:924-1062; application_controller.erl:1263-1300), kills
    // each process that is not a system process (init.erl:1072-1093), and
    // halts with the status (init.erl:943-948). `-kernel logger_level
    // warning` keeps the notice of the stop of a temporary application,
    // which has a time stamp, out of the output. The last `apply` command
    // runs `c:erlangrc/0`, which evaluates the `.erlang` file of the home
    // directory and of the user configuration directory (c.erl:1051-1072),
    // so both runs get a home directory that does not exist.
    let expected = "{controller,{registered_name,application_controller},\
        controller_linked_to_init,true,\
        kernel,[{stdlib,\"ERTS  CXC 138 10\",\"8.1\"},{kernel,\"ERTS  CXC 138 10\",\"11.0.4\"}],\
        start_kernel,{error,{already_started,kernel}},\
        start_missing,{error,{\"no such file or directory\",\"kernel_boot_missing.app\"}},\
        ensure_kernel,{ok,[]},ensure_dep,{ok,[kernel_boot_app,kernel_boot_dep]},\
        running,[{kernel_boot_dep,\"kernel boot dependency probe\",\"1\"},\
        {kernel_boot_app,\"kernel boot probe\",\"1\"},\
        {stdlib,\"ERTS  CXC 138 10\",\"8.1\"},{kernel,\"ERTS  CXC 138 10\",\"11.0.4\"}],\
        start_again,{error,{already_started,kernel_boot_app}},\
        stop_dep,ok,stop_dep_again,{error,{not_started,kernel_boot_dep}},\
        ensure_temporary,{ok,[kernel_boot_dep]},\
        ensure_missing,{error,{kernel_boot_missing,{\"no such file or directory\",\"kernel_boot_missing.app\"}}},\
        stop_badarg,{error,badarg},system,[true,false,false],system_badarg,{error,badarg},\
        status,{starting,started}}\r\n\
        {stop,kernel_boot_app,[],undefined,{stopping,started}}\r\n";
    for (arguments, code) in [("main", 0), ("main 3", 3)] {
        let (reference, status, node) =
            run_kernel_boot(arguments, true, &["{controller", "{stop,"]);
        assert_eq!(
            (reference.as_str(), status),
            (expected, Some(code)),
            "ERTS with {arguments:?}"
        );
        assert_eq!(node, expected, "PBeam with {arguments:?}");
    }
}

#[test]
fn looks_up_the_host_name_at_the_kernel_boot_as_pinned_erts() {
    // Without a domain in an inetrc file, `inet_config:init/0` looks up
    // the host name on Linux at the kernel start, when the domain is empty
    // (inet_config.erl:110-133). The native lookup opens the
    // `inet_gethost` port program of `$ROOT/erts-*/bin` with the settings
    // `[overlapped_io, {args, Args}, {packet, 4}, eof, binary]` and
    // exchanges packets with it (inet_gethost_native.erl:167-190,
    // 412-428). `kernel_boot_probe:lookup/0` then displays the domain and
    // the native lookup of the host name and of `localhost`, on each host
    // OS, and calls `init:stop/0`.
    let (reference, status, node) = run_kernel_boot("lookup", false, &["{lookup,"]);
    assert_eq!(status, Some(0), "ERTS");
    assert!(reference.starts_with("{lookup,[native],"), "{reference}");
    assert_eq!(node, reference, "PBeam");
}

/// The child side of `starts_kernel_and_stops_at_init_stop_as_pinned_erts`
/// and `looks_up_the_host_name_at_the_kernel_boot_as_pinned_erts`: boot
/// the node with `start_clean` and the `-s` arguments of the environment
/// variable [`ISOLATED`], and expect the halt of `init:stop/0,1` with the
/// status of the second argument, or 0 without it.
#[test]
#[ignore = "the kernel boot tests run it in a child process"]
fn isolated_kernel_boot() {
    let arguments = env::var(ISOLATED).expect("run it through its parent test");
    let flags = kernel_boot_flags(&arguments);
    let flags = flags.iter().map(String::as_bytes).collect::<Vec<_>>();
    let code = arguments
        .split_whitespace()
        .nth(1)
        .map_or(0, |status| status.parse().unwrap());
    assert_eq!(boot_node(&[], &flags).unwrap_err(), BeamError::Halt(code));
}

#[test]
fn stops_the_run_at_the_exit_of_init_as_pinned_erts() {
    // The emulator starts `init` as a system process (erl_init.c:355-399),
    // and the exit of a system process stops it with `System process <Pid>
    // terminated: <Reason>` (erl_process.c:14186-14188). The boot script
    // of `init_kill_probe` kills `init` from the boot process. ERTS writes
    // the text and exits with status 1, with no crash dump with
    // `ERL_CRASH_DUMP_SECONDS=0`; PBeam stops the run with the text as its
    // error (row "Node boot").
    let text = "System process <0.0.0> terminated: killed";
    let reference = Command::new("erl")
        .args(["-noshell", "-boot", KILL_BOOT_FILE, "-pa", "fixtures/boot"])
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        (
            String::from_utf8_lossy(&reference.stderr).lines().next(),
            reference.stdout.as_slice(),
            reference.status.code()
        ),
        (Some(text), &b""[..], Some(1)),
        "ERTS"
    );
    assert_eq!(
        boot_probe(KILL_PROBE, KILL_BOOT_FILE).unwrap_err(),
        BeamError::Runtime(text.into()),
        "PBeam"
    );
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
