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
    let cases: [Rejected; 10] = [
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
            &["-sname", "node", "-extra", "-sname"],
            &[],
            "the erl option -sname is not supported",
        ),
        (
            &["-args_file", "vm.args"],
            &[],
            "the erl option -args_file is not supported",
        ),
        (&["+SP", "50"], &[], "the erl option +SP is not supported"),
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
        (
            &["-noshell"],
            &[("ERL_EPMD_PORT", "")],
            "the environment variable ERL_EPMD_PORT of erl is not supported",
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
                sequential_tracer, delayed_node_table_gc, instruction_counts, \
                ultrasparc_read_tick1, no_such_item, 42, \"version\", [version], 1.0]]]), halt().";
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
                {allocator}, {allocator_sizes}, {memory_internal, a}, {foo, 1}, {}]]]]), \
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
        "alloc_util_allocators",
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
