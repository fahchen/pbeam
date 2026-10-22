//! Boot the release fixture `fixtures/release_boot` with `pbeam erl` and
//! with the pinned `erl`, and compare the results. Check the errors of the
//! `erl` options and variables that `pbeam erl` does not support.

use std::{
    os::unix::process::ExitStatusExt,
    path::PathBuf,
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
    let preloaded = preloaded_directory();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["erl", "--root", &root, "--bindir", &bindir, "--preloaded"])
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
    // arguments, and the preloaded modules that are not loaded, and stops
    // the node with `init:stop/0`, which halts with status 0.
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
        \x20             not_loaded,[],supervisor,true}\n";
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
    // (erlexec.c:661-861, 869-964, 2122-2140), and the flags of
    // `ERL_AFLAGS`, `ERL_FLAGS`, and `ERL_ZFLAGS`. `pbeam erl` has no
    // `erlexec` part for them, and no distribution for `-sname`.
    let cases: [Rejected; 6] = [
        (
            &["-sname", "node", "-extra", "-sname"],
            &[],
            "the erl option -sname is not supported",
        ),
        (
            &["-args_file", "vm.args"],
            &[],
            "the erl option -args_file is not supported",
        ),
        (&["+S", "1"], &[], "the erl option +S is not supported"),
        (
            &["-proto_dist", "-extra", "-sname", "node"],
            &[],
            "the erl option -proto_dist is not supported",
        ),
        (
            &["-noshell"],
            &[("ERL_AFLAGS", "-kernel shell_history enabled")],
            "the environment variable ERL_AFLAGS of erl is not supported",
        ),
        (
            &["-noshell"],
            &[("ERL_OTP29_FLAGS", "-kernel logger_level warning")],
            "the environment variable ERL_OTP29_FLAGS of erl is not supported",
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
