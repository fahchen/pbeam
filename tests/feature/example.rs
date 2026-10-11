//! Run the examples of `examples/` with `scripts/example.sh`, the script of
//! the mise task `example`, on the `pbeam` binary and on the pinned ERTS.
//! The tests run the script without a `mise` process. Parallel `mise`
//! processes can write `mise WARN failed to parse cache file` to stderr
//! when mise writes its cache, and stderr must be the same as ERTS.
//!
//! Each test builds its example once: the first ERTS run builds the example
//! and runs it. Each other run of the test gives `--no-build`, so it uses
//! that build, and the ERTS run and the `pbeam` run execute the same
//! compiled modules. Each example has one test, and nextest runs each test
//! in its own process, so a test run builds each example once. The script
//! renames each file of the build into `target/examples/<name>/`, so a
//! parallel build of the same example cannot give a partial file.

use std::{
    path::Path,
    process::{Command, Output, Stdio},
    time::Duration,
};

#[expect(
    dead_code,
    reason = "this test crate keeps the locale and the stdout of the script"
)]
#[path = "../common/run.rs"]
mod run;

use run::run_with_time_limit;

/// The build compiles the example and `pbeam_boot`, and starts `elixir` and
/// `erl`. The run starts the runtime of the example.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(60);

#[test]
fn runs_hello_world_with_one_command_as_pinned_erts() {
    // `bash scripts/example.sh hello_world --erts` compiles
    // `examples/hello_world/*.ex` and `pbeam_boot.erl`, and runs
    // `'Elixir.HelloWorld':main(), halt().` with `erl +S 1 -noshell`. With
    // `--no-build` and without `--erts`, it runs
    // `pbeam_boot:start('Elixir.HelloWorld', main)` with the `pbeam` binary
    // and the same build. Each runtime runs without a locale variable,
    // which gives the C locale, and with `LC_ALL=C.UTF-8`. Only the first
    // run builds.
    for (locale, erts_arguments) in [
        (None, &["hello_world", "--erts"][..]),
        (Some("C.UTF-8"), &["hello_world", "--erts", "--no-build"]),
    ] {
        let erts = run_example(erts_arguments, locale);
        let pbeam = run_example(&["hello_world", "--no-build"], locale);

        assert_eq!(
            parts(&erts),
            ("Hello, world!\\n".to_owned(), String::new(), Some(0)),
            "ERTS with LC_ALL {locale:?}"
        );
        assert_eq!(parts(&pbeam), parts(&erts), "PBeam with LC_ALL {locale:?}");
    }
}

#[test]
fn runs_fairness_as_pinned_erts_with_one_scheduler() {
    // `examples/fairness` spawns the busy processes `a` and `b`. Each one
    // counts to 20000 with no receive and reports each 10 steps. On one
    // scheduler, each turn of a process ends when it has used its 4000
    // reductions, and then the other process runs. The ERTS run uses
    // `erl +S 1`; with more schedulers, the runs change from one start to
    // the next.
    let expected: String = [
        "a 10..2660",
        "b 10..2660",
        "a 2670..5330",
        "b 2670..5330",
        "a 5340..8000",
        "b 5340..8000",
        "a 8010..10660",
        "b 8010..10660",
        "a 10670..13330",
        "b 10670..13330",
        "a 13340..16000",
        "b 13340..16000",
        "a 16010..18660",
        "b 16010..18660",
        "a 18670..20000",
        "b 18670..20000",
    ]
    .iter()
    .map(|line| format!("{line}\\n"))
    .collect();

    let erts = run_example(&["fairness", "--erts"], None);
    let pbeam = run_example(&["fairness", "--no-build"], None);

    assert_eq!(parts(&erts), (expected, String::new(), Some(0)), "ERTS");
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

#[test]
fn runs_ping_pong_as_pinned_erts() {
    // `PingPong.main/0` spawns a ping and a pong process with `spawn/1`.
    // They exchange `{:ping, from}` and `{:pong, from}` three times, and the
    // root process prints the round-trip order and the count.
    let erts = run_example(&["ping_pong", "--erts"], None);
    let pbeam = run_example(&["ping_pong", "--no-build"], None);

    assert_eq!(
        parts(&erts),
        (
            "ping 1\\npong 1\\nping 2\\npong 2\\nping 3\\npong 3\\ncount: 3\\n".to_owned(),
            String::new(),
            Some(0)
        ),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

#[test]
fn restarts_a_crashed_worker_as_pinned_erts() {
    // `CrashRestart.main/0` traps exits, links a worker with `spawn_link/1`,
    // and restarts it each time it raises, three times. Each run sets the
    // primary log level `none`, so ERTS writes no error report.
    let erts = run_example(&["crash_restart", "--erts"], Some("C.UTF-8"));
    let pbeam = run_example(&["crash_restart", "--no-build"], Some("C.UTF-8"));

    assert_eq!(
        parts(&erts),
        (
            concat!(
                "start worker 1\\n",
                "exit: worker 1 crashed\\n",
                "restart\\n",
                "start worker 2\\n",
                "exit: worker 2 crashed\\n",
                "restart\\n",
                "start worker 3\\n",
                "exit: worker 3 crashed\\n",
                "restart\\n",
                "start worker 4\\n",
                "exit: worker 4 crashed\\n",
                "stop after 4 crashes\\n",
            )
            .to_owned(),
            String::new(),
            Some(0)
        ),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

#[test]
fn dispatches_consolidated_protocols_as_pinned_erts() {
    // The script consolidates the protocols of Elixir, as `mix run` does.
    // `ProtocolDispatch.main/0` calls `String.Chars` and `Inspect` for its
    // struct `ProtocolDispatch.Point`, and `String.Chars` for an atom. A
    // consolidated protocol finds the implementation with a pattern match.
    // An unconsolidated protocol calls `Code.ensure_compiled/1`, which calls
    // `code:ensure_loaded/1`. Without consolidation, ERTS prints `false` in
    // the first two lines, and the PBeam run stops in the first dispatch
    // with status 1. The atom goes to `String.Chars.Atom` of Elixir, which
    // the example does not import, so the `pbeam` binary gets it only as an
    // implementation of the consolidated protocol.
    let erts = run_example(&["protocol_dispatch", "--erts"], None);
    let pbeam = run_example(&["protocol_dispatch", "--no-build"], None);

    assert_eq!(
        parts(&erts),
        (
            concat!(
                "String.Chars consolidated: true\\n",
                "Inspect consolidated: true\\n",
                "to_string of a point: (1, 2)\\n",
                "inspect of a point: #Point<1, 2>\\n",
                "to_string of an atom: ok\\n",
            )
            .to_owned(),
            String::new(),
            Some(0)
        ),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

#[test]
fn gives_the_modules_that_the_example_calls_through_other_modules_as_pinned_erts() {
    // `CalledModules.main/0` calls `String.length/1` of Elixir and
    // `string:length/1` of OTP, and each one calls `unicode_util:gc/1`. The
    // example does not import `unicode_util`. With only the modules that
    // the example imports, the `pbeam` run stops with
    // `unresolved external MFA unicode_util:gc/1` and status 1.
    let erts = run_example(&["called_modules", "--erts"], None);
    let pbeam = run_example(&["called_modules", "--no-build"], None);

    assert_eq!(
        parts(&erts),
        (
            "String.length: 5\\n:string.length: 5\\n".to_owned(),
            String::new(),
            Some(0)
        ),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");

    // The build writes each called module that the `pbeam` run does not get
    // into `pbeam.unloaded`: the code path has no BEAM file for it, the
    // PBeam loader does not load its BEAM file, or PBeam does not give the
    // ERTS result of the module. PBeam starts no code server, so with
    // `code_server`, `code:ensure_loaded/1` of a module that is not loaded
    // raises `badarg` where ERTS returns `{error,nofile}`
    // (`code_server.erl:84-85`, `code.erl:585-600`). Without it, the call
    // stops the run with `unresolved external MFA code_server:get_mode/0`.
    let unloaded = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("target/examples/called_modules/pbeam.unloaded"),
    )
    .unwrap();
    let entries = unloaded
        .lines()
        .map(|line| {
            let [module, file, reason] = line.split('\t').collect::<Vec<_>>()[..] else {
                panic!("an unloaded line is not <module> <file> <reason>: {line:?}");
            };
            (module, file, reason)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        entries
            .iter()
            .map(|&(module, _, reason)| (module, reason))
            .collect::<Vec<_>>(),
        [
            ("Dict", "no BEAM file in the code path"),
            (
                "Elixir.IEx.Evaluator",
                "unsupported BEAM opcode 132 (bs_match_string)"
            ),
            ("HashDict", "no BEAM file in the code path"),
            ("HashSet", "no BEAM file in the code path"),
            ("calendar", "unsupported BEAM opcode 185 (bif3)"),
            ("code_server", "PBeam has no code server"),
            ("crypto", "unsupported BEAM opcode 149 (on_load)"),
            (
                "elixir_errors",
                "unsupported BEAM opcode 132 (bs_match_string)"
            ),
            ("eval_bits", "unsupported BEAM opcode 118 (bs_get_float2)"),
            (
                "file_io_server",
                "unsupported BEAM opcode 141 (bs_skip_utf16)"
            ),
            ("inet_db", "unsupported BEAM opcode 185 (bif3)"),
            ("io_lib_fread", "unsupported BEAM opcode 185 (bif3)"),
            ("rand", "unsupported BEAM opcode 102 (fnegate)"),
            (
                "shell_docs",
                "unsupported BEAM opcode 132 (bs_match_string)"
            ),
        ]
    );

    // Each BEAM file of the list gives the load error of its reason. A
    // module that the loader learns to load fails this check, so the build
    // must then give it to the `pbeam` run.
    for (module, file, reason) in entries {
        let Some((load_error, _instruction)) = reason.split_once(" (") else {
            if module == "code_server" {
                assert!(file.ends_with("/code_server.beam"), "{file}");
            } else {
                assert_eq!(file, "", "{module}");
            }
            continue;
        };
        assert!(
            file.ends_with(&format!("/{module}.beam")),
            "{module}: {file}"
        );
        let output = Command::new(env!("CARGO_BIN_EXE_pbeam"))
            .args(["pbeam_boot", "start", "--", file])
            .output()
            .unwrap();
        assert_eq!(
            parts(&output),
            (
                String::new(),
                format!("pbeam: cannot load BEAM modules: {load_error}\\n"),
                Some(1)
            ),
            "{module}"
        );
    }
}

#[test]
fn runs_restricted_guest_with_its_strict_policy_as_pinned_erts_and_denies_other_operations() {
    // The `pbeam` run gives `--boot --policy Elixir.RestrictedGuest.Policy`,
    // so the boot of `pbeam_boot` runs with the boot authority, and the root
    // process enters the runtime with the capabilities that `initial/1` of
    // the strict Elixir policy gives `RestrictedGuest.main/0`: `process`
    // `spawn` of `RestrictedGuest.Worker`, `send`, and `monitor`, and
    // `terminal` `write`. Each worker writes to the terminal through the IO
    // service entry of `user`. The ERTS run has no policy. One test does
    // all the runs, because a build of the example by a parallel test
    // removes the lists of the build before it writes them again.
    let erts = run_example(&["restricted_guest", "--erts"], None);
    let pbeam = run_example(
        &[
            "restricted_guest",
            "--no-build",
            "--policy",
            "Elixir.RestrictedGuest.Policy",
        ],
        None,
    );

    assert_eq!(
        parts(&erts),
        (
            concat!(
                "worker 1: hello\\n",
                "worker 2: hello\\n",
                "worker 3: hello\\n",
                "main: 3 workers done\\n",
            )
            .to_owned(),
            String::new(),
            Some(0)
        ),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");

    // `restricted_guest_probe:denied/0` runs in the root process of
    // `pbeam_boot` after it enters the runtime, with the capabilities of
    // the strict policy (`restricted_guest_probe_policy`). Its spawn of
    // `RestrictedGuest.Worker` is permitted, and the worker writes to the
    // terminal. The IO requests that `terminal` `write` permits write to
    // `user` and `standard_error`. Each other operation raises
    // `error:capability_denied`: an operation that needs a capability that
    // the policy does not give, an IO request with a false `From`, an MFA
    // that is not `io_lib` `format`, `fwrite`, or `write`, an `io_lib` MFA
    // with a fun in its arguments, an input or option request, a message
    // that is not an IO request, a message to a boot process that is not an
    // IO service, also after `pbeam_cap:serve/2` of a runtime process, and
    // the output after the policy revokes `terminal`.
    let probe = run_restricted_guest(
        "restricted_guest_probe_policy",
        &["pbeam_boot", "start", "restricted_guest_probe", "denied"],
    );
    assert_eq!(
        parts(&probe),
        (
            concat!(
                "worker 0: hello\\n",
                "puts: permitted\\n",
                "format: permitted\\n",
                "permitted\\n",
                "requests: permitted\\n",
            )
            .to_owned(),
            concat!(
                "standard_error: permitted\\n",
                "pbeam: root process exited with {result,[",
                "{spawn_own_module,done},",
                "{spawn_other_module,denied},",
                "{getenv,denied},",
                "{open_port,denied},",
                "{halt,denied},",
                "{handle_other_signal,denied},",
                "{ignore_signal,denied},",
                "{exit_user,denied},",
                "{group_leader_other,denied},",
                "{request,denied},",
                "{puts,ok},",
                "{standard_error,ok},",
                "{format,ok},",
                "{write,ok},",
                "{nl,ok},",
                "{requests,ok},",
                "{empty_requests,ok},",
                "{false_from,denied},",
                "{lists_mfa,denied},",
                "{io_lib_nl,denied},",
                "{format_fun,denied},",
                "{write_fun,denied},",
                "{get_line,denied},",
                "{setopts,denied},",
                "{requests_get_line,denied},",
                "{requests_not_list,denied},",
                "{request_atom,denied},",
                "{message_atom,denied},",
                "{message_tuple,denied},",
                "{message_tag,denied},",
                "{serve_logger,ok},",
                "{request_logger,denied},",
                "{start_child,denied},",
                "{add_handler,denied},",
                "{revoke,denied},",
                "{puts_after_revoke,denied}",
                "]}\\n",
            )
            .to_owned(),
            Some(1)
        ),
        "denied operations"
    );

    // Without `terminal` `write`, `IO.puts/1` raises
    // `error:capability_denied`: the IO service entry of `user` denies the
    // request.
    let no_terminal = run_restricted_guest(
        "restricted_guest_probe_policy",
        &[
            "pbeam_boot",
            "start",
            "restricted_guest_probe",
            "no_terminal",
        ],
    );
    assert_eq!(
        parts(&no_terminal),
        (
            String::new(),
            "pbeam: root process exited with {result,[{puts,denied}]}\\n".to_owned(),
            Some(1)
        ),
        "no terminal capability"
    );

    // The strict policy gives capabilities only to `RestrictedGuest.main/0`.
    // For another entry, `initial/1` returns `deny`, so the run stops when
    // the root process enters the runtime.
    let other_entry = run_restricted_guest(
        "Elixir.RestrictedGuest.Policy",
        &["pbeam_boot", "start", "restricted_guest_probe", "denied"],
    );
    assert_eq!(
        parts(&other_entry),
        (
            String::new(),
            "pbeam: the policy denied the root process: initial/1 returned deny\\n".to_owned(),
            Some(1)
        ),
        "another entry"
    );
}

/// Run the `pbeam` binary of this test build with `--boot --policy
/// <policy>`, the `arguments`, and the BEAM files of the last build of
/// `restricted_guest`, of `restricted_guest_probe`, and of
/// `restricted_guest_probe_policy`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_restricted_guest(
///     "restricted_guest_probe_policy",
///     &["pbeam_boot", "start", "restricted_guest_probe", "denied"],
/// );
/// assert_eq!(output.status.code(), Some(1));
/// ```
fn run_restricted_guest(policy: &str, arguments: &[&str]) -> Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let beams =
        std::fs::read_to_string(root.join("target/examples/restricted_guest/pbeam.beams")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
    command
        .args(["--boot", "--policy", policy])
        .args(arguments)
        .arg("--")
        .args(beams.lines())
        .arg("fixtures/policy/restricted_guest_probe.beam")
        .arg("fixtures/policy/restricted_guest_probe_policy.beam")
        .current_dir(root);
    run_with_time_limit(&mut command, Stdio::piped(), RUN_TIME_LIMIT)
}

#[test]
fn inspects_a_tuple_as_pinned_erts() {
    // `InspectTuple.main/0` runs the last call of `TerraCounter.main/0`,
    // `IO.inspect({:ok, 1})`. `Inspect.Tuple` calls
    // `erlang:tuple_to_list/1`, `Inspect.Integer` calls
    // `erlang:integer_to_binary/2`, and `Macro.inspect_atom/2` runs
    // `String.Tokenizer.tokenize/1`, which reads
    // `elixir_config:identifier_tokenizer/0`. Without the two BIFs, the
    // `pbeam` run stops with `unresolved external MFA
    // erlang:tuple_to_list/1` and status 1.
    let erts = run_example(&["inspect_tuple", "--erts"], None);
    let pbeam = run_example(&["inspect_tuple", "--no-build"], None);

    assert_eq!(
        parts(&erts),
        ("{:ok, 1}\\n".to_owned(), String::new(), Some(0)),
        "ERTS"
    );
    assert_eq!(parts(&pbeam), parts(&erts), "PBeam");
}

#[test]
fn stops_terra_counter_when_the_local_terra_repository_is_not_at_the_pinned_commit() {
    // `scripts/terra.sh` gives the Terra sources to `terra_counter`. With
    // `PBEAM_TERRA_SRC`, it uses that local repository only when it is a git
    // repository whose HEAD is the pinned commit. Otherwise the script stops
    // with status 2 before it compiles a file, so no Terra code runs.
    let directory = std::env::temp_dir().join(format!("pbeam-terra-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(directory.join("lib")).unwrap();
    let not_git = run_terra_counter(&directory);
    assert_eq!(
        parts(&not_git),
        (
            String::new(),
            format!(
                "terra: PBEAM_TERRA_SRC {} is not the top directory of a git repository\\n",
                directory.display()
            ),
            Some(2)
        ),
        "a directory that is not a git repository"
    );

    std::fs::write(directory.join("lib/terra.ex"), "defmodule Terra do\nend\n").unwrap();
    for arguments in [
        &["init", "--quiet"][..],
        &["add", "lib/terra.ex"],
        &[
            "-c",
            "user.name=pbeam",
            "-c",
            "user.email=pbeam@example.com",
            "commit",
            "--quiet",
            "--message",
            "stub",
        ],
    ] {
        let status = Command::new("git")
            .arg("-C")
            .arg(&directory)
            .args(arguments)
            .status()
            .unwrap();
        assert!(status.success(), "git {arguments:?}: {status}");
    }
    let head = Command::new("git")
        .arg("-C")
        .arg(&directory)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    let head = String::from_utf8(head.stdout).unwrap();
    let other_commit = run_terra_counter(&directory);
    std::fs::remove_dir_all(&directory).unwrap();

    assert_eq!(
        parts(&other_commit),
        (
            String::new(),
            format!(
                "terra: {} is at {}, not at the pinned commit 30dea5667d7e4ae5777aa3df77acb36c4aac6967\\n",
                directory.display(),
                head.trim_end()
            ),
            Some(2)
        ),
        "a git repository at another commit"
    );
}

/// Run `scripts/example.sh terra_counter --erts` with the local Terra
/// repository `terra` in `PBEAM_TERRA_SRC`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_terra_counter(Path::new("/tmp/terra"));
/// assert_eq!(output.status.code(), Some(2));
/// ```
fn run_terra_counter(terra: &Path) -> Output {
    let mut command = Command::new("bash");
    command
        .args(["scripts/example.sh", "terra_counter", "--erts"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PBEAM_TERRA_SRC", terra);
    run_with_time_limit(&mut command, Stdio::piped(), RUN_TIME_LIMIT)
}

/// Run `scripts/example.sh` with `arguments`, with no locale variable except
/// `LC_ALL=locale`. `PBEAM_BIN` gives the `pbeam` binary of this test
/// build, which `cargo llvm-cov` measures, so the script does not start
/// `cargo run`. The script finds `elixirc`, `erlc`, `elixir`, and `erl` in
/// `PATH`, as the other tests find `erl`.
///
/// # Examples
///
/// ```rust,ignore
/// let output = run_example(&["hello_world", "--erts"], Some("C.UTF-8"));
/// assert_eq!(output.status.code(), Some(0));
/// ```
fn run_example(arguments: &[&str], locale: Option<&str>) -> Output {
    let mut command = Command::new("bash");
    command
        .arg("scripts/example.sh")
        .args(arguments)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PBEAM_BIN", env!("CARGO_BIN_EXE_pbeam"))
        .env_remove("LANG")
        .env_remove("LC_CTYPE")
        .env_remove("LC_ALL");
    if let Some(locale) = locale {
        command.env("LC_ALL", locale);
    }
    run_with_time_limit(&mut command, Stdio::piped(), RUN_TIME_LIMIT)
}

/// Return the stdout, the stderr, and the exit status of a run. The output
/// is escaped as ASCII, so a failure message shows each byte.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(parts(&output), ("Hello, world!\\n".to_owned(), String::new(), Some(0)));
/// ```
fn parts(output: &Output) -> (String, String, Option<i32>) {
    (
        output.stdout.escape_ascii().to_string(),
        output.stderr.escape_ascii().to_string(),
        output.status.code(),
    )
}
