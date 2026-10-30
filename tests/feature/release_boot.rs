//! Boot the release fixture `fixtures/release_boot` with `pbeam erl` and
//! with the pinned `erl`, and compare the results. Check the errors of the
//! `erl` options and variables that `pbeam erl` does not support.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

#[expect(
    dead_code,
    reason = "this test crate gives the preloaded directory, not bytes, to the binary"
)]
#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam_path;

/// The `erl` arguments of the release fixture, in the form that
/// `bin/<app> start` of a Mix release gives `erl`: embedded mode, the
/// `sys` configuration, the `start` boot script, and the `RELEASE_LIB` boot
/// variable, then `-s` and the `-extra` arguments. `-sname` after `-extra`
/// is a plain argument (erlexec.c:637-640).
const RELEASE_ARGUMENTS: [&str; 17] = [
    "-noshell",
    "-mode",
    "embedded",
    "-config",
    "fixtures/release_boot/sys",
    "-boot",
    "fixtures/release_boot/start",
    "-boot_var",
    "RELEASE_LIB",
    "fixtures",
    "-s",
    "release_boot_probe",
    "main",
    "-extra",
    "plain",
    "-sname",
    "x",
];

/// The variables of `erlexec` in the environment, which a run of the tests
/// removes.
const ERL_VARIABLES: [&str; 5] = [
    "ERL_OTP29_FLAGS",
    "ERL_AFLAGS",
    "ERL_FLAGS",
    "ERL_ZFLAGS",
    "ERL_EPMD_PORT",
];

/// Return the `root` and `bindir` flags that `erl` gives the pinned
/// emulator, as `init:get_argument/1` gives them.
fn emulator_flags() -> (String, String) {
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
    let text = String::from_utf8(output.stdout).unwrap();
    let (root, bindir) = text.split_once('\n').unwrap();
    (root.to_owned(), bindir.to_owned())
}

/// Make `command` run with a home directory and a user configuration
/// directory that do not exist, so `c:erlangrc/0` and the shell history
/// read no file of the host user, without the variables of
/// [`ERL_VARIABLES`], and with stdin from `/dev/null`.
fn isolate(command: &mut Command) -> &mut Command {
    for variable in ERL_VARIABLES {
        command.env_remove(variable);
    }
    command
        .env("HOME", "fixtures/boot/no_home")
        .env("XDG_CONFIG_HOME", "fixtures/boot/no_home")
        .stdin(Stdio::null())
}

/// Run `pbeam erl` with the root, the bin directory, and the preloaded
/// directory of the pinned install, and the `erl` arguments `arguments`.
fn pbeam_erl(arguments: &[&str], environment: &[(&str, &str)]) -> Output {
    let (root, bindir) = emulator_flags();
    pbeam_erl_in(&root, Path::new(&bindir), arguments, environment)
}

/// Run `pbeam erl` with the root `root`, the bin directory `bindir`, the
/// preloaded directory of the pinned install, and the `erl` arguments
/// `arguments`.
fn pbeam_erl_in(
    root: &str,
    bindir: &Path,
    arguments: &[&str],
    environment: &[(&str, &str)],
) -> Output {
    let preloaded = preloaded_directory();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["erl", "--root", root, "--bindir"])
        .arg(bindir)
        .arg("--preloaded")
        .arg(preloaded)
        .arg("--")
        .args(arguments);
    isolate(&mut command).envs(environment.iter().copied());
    command.output().unwrap()
}

/// Return the `ebin` directory of `erts` of the pinned install, which holds
/// the BEAM files of the preloaded modules.
fn preloaded_directory() -> PathBuf {
    pinned_beam_path("erlang").parent().unwrap().to_owned()
}

#[test]
fn boots_a_release_with_the_arguments_of_erl_as_pinned_erts() {
    // The boot script of the fixture loads kernel, stdlib, and the probe
    // with `primLoad` in embedded mode, and starts each application with
    // `application:start_boot/2` (init.erl:1317-1358). `-config` gives
    // the environment of `sys.config` to `application_controller`
    // (application_controller.erl:1964-2021), and `-s` runs
    // `release_boot_probe:main/0` after the boot (init.erl:1530-1546). The
    // probe displays the applications, the environment, the mode, the plain
    // arguments, the preloaded modules that are not loaded, and whether
    // `PATH` starts with the bindir, as erlexec sets it
    // (erlexec.c:556-620), and stops the node with `init:stop/0`, which
    // halts with status 0.
    let mut reference = Command::new("erl");
    reference.args(RELEASE_ARGUMENTS);
    let reference = isolate(&mut reference).output().unwrap();
    let expected = "{release_boot,applications,\n\
        \x20             [{release_boot_probe,\"release boot probe\",\"1\"},\n\
        \x20              {stdlib,\"ERTS  CXC 138 10\",\"8.1\"},\n\
        \x20              {kernel,\"ERTS  CXC 138 10\",\"11.0.4\"}],\n\
        \x20             greeting,\n\
        \x20             {ok,\"hello from sys.config\"},\n\
        \x20             mode,embedded,plain,\n\
        \x20             [\"plain\",\"-sname\",\"x\"],\n\
        \x20             not_loaded,[],supervisor,true,path_bindir,true}\n";
    let result = |output: &Output| {
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    };
    let expected = (expected.to_owned(), String::new(), Some(0));
    assert_eq!(result(&reference), expected, "ERTS");
    assert_eq!(
        result(&pbeam_erl(&RELEASE_ARGUMENTS, &[])),
        expected,
        "PBeam"
    );
}

/// The flag variables of the release boot test of the flag variables. The
/// `-eval` of `ERL_AFLAGS` runs before the `-s` of the release and writes
/// the kernel flags, the two kernel variables that they set, the online
/// schedulers, and the plain arguments.
const FLAG_VARIABLES: [(&str, &str); 4] = [
    ("ERL_OTP29_FLAGS", "-kernel logger_level warning"),
    (
        "ERL_AFLAGS",
        "-kernel shell_history enabled -eval 'io:format(\"~p~n\", [{init:get_argument(kernel), \
         application:get_env(kernel, shell_history), application:get_env(kernel, logger_level), \
         erlang:system_info(schedulers_online), init:get_plain_arguments()}])'",
    ),
    ("ERL_FLAGS", "+S 1"),
    ("ERL_ZFLAGS", "-extra \"z w\" s\\ t"),
];

#[test]
fn reads_the_flag_variables_of_erlexec_as_pinned_erts() {
    // `erlexec` adds the words of `ERL_OTP29_FLAGS` and `ERL_AFLAGS` before
    // the command line and those of `ERL_FLAGS` and `ERL_ZFLAGS` after it,
    // and moves the `-extra` words of each to the end
    // (erlexec.c:2177-2260). It splits a value at white space, and a quote
    // or a backslash keeps white space in a word (erlexec.c:1770-1931).
    let mut reference = Command::new("erl");
    reference.args(RELEASE_ARGUMENTS);
    let reference = isolate(&mut reference)
        .envs(FLAG_VARIABLES)
        .output()
        .unwrap();
    let result = |output: &Output| {
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
            output.status.code(),
        )
    };
    let (stdout, stderr, status) = result(&reference);
    assert_eq!((stderr.as_str(), status), ("", Some(0)), "ERTS: {stdout}");
    assert!(
        stdout.starts_with(
            "{{ok,[[\"logger_level\",\"warning\"],[\"shell_history\",\"enabled\"]]},\n \
             {ok,enabled},\n {ok,warning},\n 1,\n \
             [\"plain\",\"-sname\",\"x\",\"z w\",\"s t\"]}\n"
        ),
        "ERTS: {stdout}"
    );
    assert_eq!(
        result(&pbeam_erl(&RELEASE_ARGUMENTS, &FLAG_VARIABLES)),
        (stdout, stderr, status),
        "PBeam"
    );
}

/// A rejected command of `pbeam erl`: the `erl` arguments, the
/// environment, and the message of the error.
type Rejected = (
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
    &'static str,
);

#[test]
fn rejects_the_options_and_variables_of_erlexec() {
    // `erlexec` reads these options before it starts the emulator
    // (erlexec.c:661-861, 869-964, 2122-2140). `pbeam erl` has no
    // `erlexec` part for them.
    let cases: [Rejected; 7] = [
        (
            &["-S", "release_boot_probe", "main", "--", "x"],
            &[],
            "the erl option -S is not supported",
        ),
        (
            &["-emu_type", "debug"],
            &[],
            "the erl option -emu_type is not supported",
        ),
        (
            &["-smp", "disable"],
            &[],
            "the erl option -smp is not supported",
        ),
        (
            &["-args_file", "vm.args"],
            &[],
            "the erl option -args_file is not supported",
        ),
        (&["+SP", "50"], &[], "the erl option +SP is not supported"),
        // A flag of a flag variable gives the error of the same flag on
        // the command line.
        (
            &["-noshell"],
            &[("ERL_FLAGS", "-env NAME value")],
            "the erl option -env is not supported",
        ),
        (
            &["-noshell", "-extra", "plain"],
            &[("ERL_ZFLAGS", "+SP 50")],
            "the erl option +SP is not supported",
        ),
    ];
    for (arguments, environment, message) in cases {
        let output = pbeam_erl(arguments, environment);
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stderr).into_owned(),
                output.status.code()
            ),
            (
                format!("pbeam: {message}: see \"Release boot\" in docs/erts-differences.md\n"),
                Some(1)
            ),
            "{arguments:?}"
        );
    }
}

/// The distribution options of `erl`. `-kernel start_distribution false`
/// keeps the kernel from the start of `net_kernel`
/// (kernel.erl:206-208), and `-start_epmd false` keeps `erlexec` from the
/// start of epmd. `-epmd` leaves the arguments of `init`.
const DISTRIBUTION_ARGUMENTS: [&str; 33] = [
    "-noshell",
    "-sname",
    "a",
    "-name",
    "b@host",
    "-setcookie",
    "c",
    "-hidden",
    "-connect_all",
    "false",
    "-proto_dist",
    "inet_tcp",
    "-no_epmd",
    "-epmd_module",
    "m",
    "-epmd",
    "unused",
    "-start_epmd",
    "false",
    "-dist_listen",
    "false",
    "-kernel",
    "start_distribution",
    "false",
    "-kernel",
    "inet_dist_listen_min",
    "9100",
    "-kernel",
    "inet_dist_listen_max",
    "9200",
    "-eval",
    "io:format(\"~p~n\", [init:get_arguments()]), halt().",
    "-extra",
];

/// Return the standard output, the standard error, and the exit status of
/// `output`.
fn result(output: &Output) -> (String, String, Option<i32>) {
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code(),
    )
}

#[test]
fn gives_the_distribution_options_to_init_as_pinned_erts() {
    // `erlexec` gives `-name`, `-sname`, `-proto_dist`, `-no_epmd`, and
    // `-start_epmd` to `init`, keeps `-epmd` for itself
    // (erlexec.c:728-731, 774-805, 812-851), and gives `-epmd_port` of
    // `ERL_EPMD_PORT` after `-home` (erlexec.c:625-635, 1754-1760). The
    // other options go to `init` unchanged.
    let environment = [("ERL_EPMD_PORT", "4370")];
    let mut reference = Command::new("erl");
    reference.args(DISTRIBUTION_ARGUMENTS);
    let reference = isolate(&mut reference).envs(environment).output().unwrap();
    let (stdout, stderr, status) = result(&reference);
    assert_eq!((stderr.as_str(), status), ("", Some(0)), "ERTS: {stdout}");
    assert!(
        stdout.contains(
            " {home,[\"fixtures/boot/no_home\"]},\n {epmd_port,[\"4370\"]},\n {noshell,[]},\n \
             {sname,[\"a\"]},\n {name,[\"b@host\"]},\n {setcookie,[\"c\"]},\n {hidden,[]},\n \
             {connect_all,[\"false\"]},\n {proto_dist,[\"inet_tcp\"]},\n {no_epmd,[]},\n \
             {epmd_module,[\"m\"]},\n {start_epmd,[\"false\"]},\n"
        ),
        "ERTS: {stdout}"
    );
    assert_eq!(
        result(&pbeam_erl(&DISTRIBUTION_ARGUMENTS, &environment)),
        (stdout, stderr, status),
        "PBeam"
    );
}

/// Return a new empty directory of the test temporary directory.
fn empty_directory(name: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("release-boot-{name}"));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn starts_the_epmd_program_as_pinned_erts() {
    // With `-sname` and without `-start_epmd false`, `erlexec` runs the
    // command of `-epmd` with `system(3)` before the emulator starts
    // (erlexec.c:1131-1132, 1327-1372).
    let directory = empty_directory("epmd-program");
    let mark = directory.join("started");
    let program = format!("printf %s \"$0\" > {}", mark.display());
    let arguments = [
        "-noshell",
        "-sname",
        "a",
        "-epmd",
        &program,
        "-kernel",
        "start_distribution",
        "false",
        "-eval",
        "halt().",
    ];
    let mut reference = Command::new("erl");
    reference.args(arguments);
    let reference = isolate(&mut reference).output().unwrap();
    assert_eq!(result(&reference), (String::new(), String::new(), Some(0)));
    assert_eq!(fs::read_to_string(&mark).unwrap(), "sh", "ERTS");
    fs::remove_file(&mark).unwrap();
    assert_eq!(
        result(&pbeam_erl(&arguments, &[])),
        (String::new(), String::new(), Some(0)),
        "PBeam"
    );
    assert_eq!(fs::read_to_string(&mark).unwrap(), "sh", "PBeam");
}

#[test]
fn starts_epmd_of_the_path_for_a_distributed_node() {
    // `erlexec` runs `$BINDIR/epmd -daemon` (erlexec.c:1336-1343). `pbeam
    // erl` runs `epmd -daemon` of `PATH`, which has the bin directory
    // first (erlexec.c:556-620). A fake `epmd` in the bin directory writes
    // its arguments. `-start_epmd false` and `-no_epmd` turn the start off
    // (erlexec.c:791-794, 835-846).
    let (root, _) = emulator_flags();
    let bindir = empty_directory("epmd-bindir");
    let mark = bindir.join("arguments");
    let epmd = bindir.join("epmd");
    fs::write(
        &epmd,
        format!("#!/bin/sh\nprintf %s \"$*\" > {}\n", mark.display()),
    )
    .unwrap();
    fs::set_permissions(&epmd, fs::Permissions::from_mode(0o755)).unwrap();
    let start = |options: &[&str]| {
        let mut arguments = vec!["-noshell"];
        arguments.extend(options);
        arguments.extend(["-kernel", "start_distribution", "false", "-eval", "halt()."]);
        let output = pbeam_erl_in(&root, &bindir, &arguments, &[]);
        assert_eq!(result(&output), (String::new(), String::new(), Some(0)));
        fs::read_to_string(&mark).ok()
    };
    assert_eq!(start(&["-name", "a@host"]).as_deref(), Some("-daemon"));
    fs::remove_file(&mark).unwrap();
    assert_eq!(start(&["-sname", "a", "-start_epmd", "false"]), None);
    assert_eq!(
        start(&["-sname", "a", "-no_epmd", "-proto_dist", "inet_tcp"]),
        None
    );
    assert_eq!(start(&["-start_epmd", "true"]), None);
}

#[test]
fn stops_without_an_epmd_executable() {
    // PBeam needs `epmd` in `PATH` for `-sname` (row "Distribution" of
    // docs/erts-differences.md). The error comes before the boot.
    let output = pbeam_erl_in(
        "/nonexistent",
        Path::new("/nonexistent/bin"),
        &["-sname", "a"],
        &[("PATH", "/nonexistent")],
    );
    assert_eq!(
        result(&output),
        (
            String::new(),
            "pbeam: cannot start epmd -daemon: No such file or directory (os error 2): see \
             \"Distribution\" in docs/erts-differences.md\n"
                .to_owned(),
            Some(1)
        )
    );
}

#[test]
fn rejects_a_bad_distribution_option_as_pinned_erts() {
    // erlexec.c:412-418 and 1303-1308 write `Missing argument(s) for
    // '<Option>'.` and the usage text, erlexec.c:838-846 the error of
    // `-start_epmd`, and erlexec.c:1118-1123 the error of `-no_epmd`
    // without `-proto_dist`, and exit with status 1.
    let cases: [(&[&str], &str); 9] = [
        (&["-sname"], "Missing argument(s) for '-sname'."),
        // `erlexec` reads its options before the emulator reads `+S`.
        (
            &["+S", "9999", "-sname"],
            "Missing argument(s) for '-sname'.",
        ),
        (
            &["+S", "9999", "-no_epmd"],
            "Missing -proto_dist option, expected when using -no_epmd.",
        ),
        (&["-name", "--", "a"], "Missing argument(s) for '-name'."),
        (&["-proto_dist"], "Missing argument(s) for '-proto_dist'."),
        (&["-epmd"], "Missing argument(s) for '-epmd'."),
        (&["-start_epmd"], "Missing argument(s) for '-start_epmd'."),
        (
            &["-start_epmd", "yes"],
            "Expected boolean argument for '-start_epmd'.",
        ),
        (
            &["-no_epmd", "-eval", "halt()."],
            "Missing -proto_dist option, expected when using -no_epmd.",
        ),
    ];
    for (arguments, expected) in cases {
        let mut reference = Command::new("erl");
        reference.args(arguments);
        let reference = isolate(&mut reference).output().unwrap();
        for output in [&reference, &pbeam_erl(arguments, &[])] {
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            // PBeam writes its `pbeam: ` prefix and no usage text, and
            // ERTS writes `erlexec: ` before the error of `-no_epmd`.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first = stderr.lines().next().map(|line| {
                line.trim_start_matches("pbeam: ")
                    .trim_start_matches("erlexec: ")
            });
            assert_eq!(first, Some(expected), "{arguments:?}: {output:?}");
        }
    }
}

#[test]
fn reports_a_preloaded_module_that_cannot_be_read() {
    // ERTS builds the preloaded modules into the emulator. `pbeam erl`
    // reads them from `--preloaded`, so a directory without them stops the
    // command before the boot.
    let (root, bindir) = emulator_flags();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command.args([
        "erl",
        "--root",
        &root,
        "--bindir",
        &bindir,
        "--preloaded",
        "fixtures/release_boot",
        "--",
        "-noshell",
    ]);
    let output = isolate(&mut command).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with(
            "pbeam: cannot load the preloaded modules: cannot read fixtures/release_boot/erts_code_purger.beam: "
        ),
        "{stderr}"
    );
    assert_eq!(
        (output.status.code(), output.status.signal()),
        (Some(1), None)
    );
}

#[test]
fn halts_with_the_init_crash_text_when_the_eval_fails() {
    // `start_it/1` writes `Error! Failed to eval: <Expression>` and raises
    // again (init.erl:1552-1567). At the boot process exit, `init` calls
    // `crash("Runtime terminating during boot", [Reason])`, which makes the
    // text with `erts_internal:term_to_string(Reason, 32768)` and halts
    // with it as the slogan (init.erl:665-679, 710-712, bif.c:4372-4386):
    // status 1. With `ERL_CRASH_DUMP_SECONDS=0`, ERTS writes no crash dump,
    // and PBeam never writes one (row "Run end").
    let arguments = ["-noshell", "-eval", "foo:bar()"];
    let mut erts = Command::new("erl");
    erts.args(arguments);
    let erts = isolate(&mut erts)
        .env("ERL_CRASH_DUMP_SECONDS", "0")
        .output()
        .unwrap();
    let pbeam = pbeam_erl(&arguments, &[]);
    let stderr = "Error! Failed to eval: foo:bar()\r\n\r\nRuntime terminating during boot \
                  ({undef,[{foo,bar,[],[]},{erl_eval,do_apply,7,[{file,\"erl_eval.erl\"},{line,1044}]},\
                  {init,start_it,1,[]},{init,start_em,1,[]},{init,do_boot,3,[]}]})\r\n";
    for (name, output) in [("ERTS", erts), ("PBeam", pbeam)] {
        assert_eq!(
            (
                String::from_utf8_lossy(&output.stdout).as_ref(),
                String::from_utf8_lossy(&output.stderr).as_ref(),
                output.status.code()
            ),
            ("", stderr, Some(1)),
            "{name}"
        );
    }
}

/// The `-eval` expression of the scheduler probes: the total and the
/// online count of the schedulers.
const SCHEDULERS_EVAL: &str = "io:format(\"~p ~p~n\", [erlang:system_info(schedulers), \
     erlang:system_info(schedulers_online)]), halt().";

/// Run the scheduler probe with the `+S` value `value` on `pbeam erl` and
/// on the pinned `erl`.
fn schedulers_probe(value: &str) -> (Output, Output) {
    let arguments = ["+S", value, "-noshell", "-eval", SCHEDULERS_EVAL];
    let mut reference = Command::new("erl");
    reference.args(arguments);
    let reference = isolate(&mut reference).output().unwrap();
    (pbeam_erl(&arguments, &[]), reference)
}

#[test]
fn sets_the_scheduler_counts_of_the_s_flag_as_pinned_erts() {
    // `+S T:O` sets the total and the online count (erl_init.c:1103-1149).
    for (value, expected) in [("2", "2 2\n"), ("4:2", "4 2\n")] {
        let (actual, reference) = schedulers_probe(value);
        assert_eq!(
            String::from_utf8_lossy(&reference.stdout),
            expected,
            "{reference:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&actual.stdout),
            expected,
            "{actual:?}"
        );
        assert!(actual.status.success(), "{actual:?}");
    }
    // `+S :O` keeps the default total. ERTS takes the configured CPUs and
    // PBeam takes `available_parallelism`, so the probe checks PBeam
    // against the host count (row "Scheduling" of docs/erts-differences.md).
    let host = std::thread::available_parallelism().unwrap().get();
    let (actual, _) = schedulers_probe(":1");
    assert_eq!(
        String::from_utf8_lossy(&actual.stdout),
        format!("{host} 1\n"),
        "{actual:?}"
    );
}

/// The `-eval` expression of the scheduler item probe: the value of each
/// scheduler item of `erlang:system_info/1`. The probe leaves out
/// `scheduler_id`, which depends on the scheduler that runs the probe.
fn scheduler_items_eval() -> String {
    "io:format(\"~p~n\", [[erlang:system_info(I) || I <- [schedulers_online, \
     dirty_cpu_schedulers, dirty_cpu_schedulers_online, dirty_io_schedulers, \
     multi_scheduling, multi_scheduling_blockers, normal_multi_scheduling_blockers, \
     scheduler_bindings, scheduler_bind_type, cpu_topology, \
     logical_processors_available, system_version]]]), halt()."
        .to_string()
}

#[test]
fn gives_the_scheduler_items_of_system_info_as_pinned_erts() {
    // erl_bif_info.c:2790-2798, 3111-3113, 3212-3221, 3251-3254,
    // 3337-3348, 3390-3399; erl_misc_utils.c:841-1031 on Linux;
    // system_version: 92-160, 492-523.
    let eval = scheduler_items_eval();
    for value in ["1", "2", "4:2"] {
        let arguments = ["+S", value, "-noshell", "-eval", eval.as_str()];
        let mut reference = Command::new("erl");
        reference.args(arguments);
        let reference = isolate(&mut reference).output().unwrap();
        let actual = pbeam_erl(&arguments, &[]);
        assert!(reference.status.success(), "{reference:?}");
        assert_eq!(
            String::from_utf8_lossy(&actual.stdout),
            String::from_utf8_lossy(&reference.stdout),
            "+S {value}: {actual:?}"
        );
    }
}

#[test]
fn gives_the_other_items_of_system_info_and_badarg_as_pinned_erts() {
    // erl_bif_info.c:2780-2783, 2855-2856, 2899-2904, 3042-3054,
    // 3179-3190, 3425-3433, 3633; sys/unix/sys.c:838-885.
    let eval = "io:format(\"~p~n\", [[try erlang:system_info(I) catch C:R -> {C, R} end \
                || I <- [version, machine, os_version, start_time, creation, break_ignored, \
                sequential_tracer, delayed_node_table_gc, alloc_util_allocators, \
                instruction_counts, ultrasparc_read_tick1, no_such_item, 42, \"version\", \
                [version], 1.0]]]), halt().";
    let arguments = ["-noshell", "-eval", eval];
    let mut reference = Command::new("erl");
    reference.args(arguments);
    let reference = isolate(&mut reference).output().unwrap();
    let actual = pbeam_erl(&arguments, &[]);
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&reference.stdout),
        "{actual:?}"
    );
}

#[test]
fn gives_snifs_and_the_tuple_items_of_system_info_as_pinned_erts() {
    // erl_bif_info.c:2489-2640 (info_1_tuple), 2763-2771 and 2890-2898
    // (snifs); erl_cpu_topology.c:1557-1589, 1619-1638.
    let eval = "io:format(\"~p~n\", [[erlang:system_info(snifs) | \
                [try erlang:system_info(I) catch C:R -> {C, R} end \
                || I <- [{wordsize, internal}, {wordsize, external}, {wordsize, x}, \
                {wordsize}, {wordsize, internal, 1}, {cpu_topology, defined}, \
                {cpu_topology, detected}, {cpu_topology, used}, {cpu_topology, x}, \
                {cpu_topology}, {internal_cpu_topology, defined}, \
                {internal_cpu_topology, detected}, {internal_cpu_topology, used}, \
                {memory_checker, test_leak}, {memory_checker, x}, {memory_checker}, \
                {allocator}, {allocator_sizes}, {memory_internal, a}, {foo, 1}, {1, a}, {}]]]]), \
                halt().";
    let arguments = ["-noshell", "-eval", eval];
    let mut reference = Command::new("erl");
    reference.args(arguments);
    let reference = isolate(&mut reference).output().unwrap();
    let actual = pbeam_erl(&arguments, &[]);
    assert!(reference.status.success(), "{reference:?}");
    assert_eq!(
        String::from_utf8_lossy(&actual.stdout),
        String::from_utf8_lossy(&reference.stdout),
        "{actual:?}"
    );
}

#[test]
fn rejects_the_allocator_items_of_system_info() {
    // PBeam has no ERTS memory allocators (erl_bif_info.c:2499-2538), and
    // ERTS crashes on {internal_cpu_topology, x} and reads past an array
    // on {memory_checker, test_overflow} (erl_bif_info.c:2539-2556).
    for item in [
        "{allocator, binary_alloc}",
        "{allocator, binary_alloc, x}",
        "{allocator_sizes, binary_alloc}",
        "{memory_internal, a, b}",
        "{internal_cpu_topology, x}",
        "{memory_checker, test_overflow}",
        "allocated_areas",
    ] {
        let eval = format!("erlang:system_info({item}), halt().");
        let actual = pbeam_erl(&["-noshell", "-eval", eval.as_str()], &[]);
        let stderr = String::from_utf8_lossy(&actual.stderr);
        assert!(
            stderr.contains("unsupported erlang:system_info/1 argument"),
            "{item}: {actual:?}"
        );
    }
}

#[test]
fn rejects_system_version_with_a_thread_pool_size() {
    // ERTS reads ERL_THREAD_POOL_SIZE (erl_init.c:873-876); PBeam has no
    // async threads, so it gives a clear runtime error.
    let eval = "erlang:system_info(system_version), halt().";
    let actual = pbeam_erl(
        &["-noshell", "-eval", eval],
        &[("ERL_THREAD_POOL_SIZE", "4")],
    );
    let stderr = String::from_utf8_lossy(&actual.stderr);
    assert!(
        stderr.contains("unsupported erlang:system_info(system_version) with ERL_THREAD_POOL_SIZE"),
        "{actual:?}"
    );
}

#[test]
fn rejects_a_bad_s_flag_as_pinned_erts() {
    // erl_init.c:1103-1149 writes the error, then the usage text, and
    // exits with status 1.
    for (value, expected) in [
        (
            "3:5",
            "bad amount of schedulers online 5 (total amount of schedulers 3)",
        ),
        ("x", "bad amount of schedulers x"),
    ] {
        let (actual, reference) = schedulers_probe(value);
        for output in [&actual, &reference] {
            assert_eq!(output.status.code(), Some(1), "{output:?}");
            // PBeam writes its `pbeam: ` prefix and no usage text.
            let stderr = String::from_utf8_lossy(&output.stderr);
            let first = stderr
                .lines()
                .next()
                .map(|line| line.trim_start_matches("pbeam: "));
            assert_eq!(first, Some(expected), "{output:?}");
        }
    }
}
