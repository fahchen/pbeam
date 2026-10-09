use std::{
    env,
    ffi::c_int,
    fs,
    os::unix::process::ExitStatusExt,
    process::{Command, Stdio},
};

use pbeam_runtime::{BeamProgram, install_start_dispositions};

const SIGNAL_PROBE: &str = "fixtures/signal/signal_probe.beam";

/// The environment variable that gives the `isolated_set_signal` child the
/// file for its result.
const RESULT: &str = "PBEAM_SIGNAL_RESULT";

/// Each signal of `signalterm_to_signum` (sys.c:697-722) on this host, with
/// its disposition after `signal_probe:main/0`. SIGSTOP is not in the list:
/// the host does not let a process change it, ERTS ignores the `sigaction`
/// error (sys.c:437-460), and macOS gives `EINVAL` also for a read of it.
const DISPOSITIONS: &[(&str, c_int, &str)] = &[
    ("sighup", libc::SIGHUP, "ignore"),
    ("sigquit", libc::SIGQUIT, "ignore"),
    ("sigabrt", libc::SIGABRT, "ignore"),
    ("sigalrm", libc::SIGALRM, "ignore"),
    ("sigterm", libc::SIGTERM, "ignore"),
    ("sigusr1", libc::SIGUSR1, "default"),
    ("sigusr2", libc::SIGUSR2, "handle"),
    ("sigchld", libc::SIGCHLD, "default"),
    ("sigtstp", libc::SIGTSTP, "ignore"),
    ("sigcont", libc::SIGCONT, "handle"),
    ("sigwinch", libc::SIGWINCH, "handle"),
    #[cfg(target_os = "macos")]
    ("siginfo", libc::SIGINFO, "handle"),
];

/// Return the results of `signal_probe:main/0` on this host. `siginfo` is
/// a signal only on a host with `SIGINFO` (sys.c:716-718), which of the
/// PBeam hosts is only macOS.
fn expected_results() -> Vec<&'static str> {
    let siginfo = if cfg!(target_os = "macos") {
        "ok"
    } else {
        "badarg"
    };
    let mut results = vec!["ok"; 13];
    results.push(siginfo);
    results.extend(["badarg"; 9]);
    results
}

#[test]
fn sets_the_host_signal_disposition_with_os_set_signal() {
    let results = expected_results();
    let erts = Command::new("erl")
        .args([
            "-noinput",
            "-pa",
            "fixtures/signal",
            "-eval",
            "io:format(standard_error, \"~w~n\", [signal_probe:main()]), halt().",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(erts.status.success());
    assert!(erts.stdout.is_empty());
    assert_eq!(
        String::from_utf8(erts.stderr).unwrap(),
        format!("[{}]\n", results.join(","))
    );

    let directory = env::temp_dir().join(format!("pbeam-signal-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let result = directory.join("result");
    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "isolated_set_signal",
            "--include-ignored",
            "--quiet",
        ])
        .env(RESULT, &result)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let written = fs::read_to_string(&result);
    fs::remove_dir_all(&directory).unwrap();
    assert!(child.status.success(), "child exited with {}", child.status);
    let atoms: Vec<String> = results
        .iter()
        .map(|name| format!("Atom({name:?})"))
        .collect();
    let dispositions: Vec<String> = DISPOSITIONS
        .iter()
        .map(|(name, _, disposition)| format!("{name}={disposition}"))
        .collect();
    assert_eq!(
        written.unwrap(),
        format!(
            "Ok(List([{}]))\n{}",
            atoms.join(", "),
            dispositions.join(" ")
        )
    );
}

/// The child side of `sets_the_host_signal_disposition_with_os_set_signal`:
/// run `signal_probe:main/0`, send SIGHUP and SIGUSR2 to this process, and
/// write the `Debug` text of the result and the disposition of each signal
/// to the result file. A child process is necessary, because a disposition
/// is the same for all threads of a process, and for example `SIGCHLD` with
/// `SIG_IGN` changes the `Command` waits of other tests. The child inherits
/// `LLVM_PROFILE_FILE`, so `cargo llvm-cov` measures it.
#[test]
#[ignore = "sets_the_host_signal_disposition_with_os_set_signal runs it in a child process"]
fn isolated_set_signal() {
    let probe = fs::read(SIGNAL_PROBE).unwrap();
    let program = BeamProgram::load(&[&probe]).unwrap();
    let result = program.execute("signal_probe", "main", &[]);
    // SAFETY: `raise` runs the disposition of the signal in this thread.
    // `main/0` ignores SIGHUP and gives SIGUSR2 a handler, so both keep the
    // process alive, as the ERTS `ignore` and `handle` modes do.
    unsafe {
        assert_eq!(libc::raise(libc::SIGHUP), 0);
        assert_eq!(libc::raise(libc::SIGUSR2), 0);
    }
    let dispositions: Vec<String> = DISPOSITIONS
        .iter()
        .map(|(name, signal, _)| format!("{name}={}", disposition(*signal)))
        .collect();
    fs::write(
        env::var_os(RESULT).unwrap(),
        format!("{result:?}\n{}", dispositions.join(" ")),
    )
    .unwrap();
}

/// Read the disposition of `signal` with `sigaction`: `ignore` for
/// `SIG_IGN`, `default` for `SIG_DFL`, and `handle` for a handler.
fn disposition(signal: c_int) -> &'static str {
    // SAFETY: `sigaction` is a C struct of integers and a signal set, so all
    // zeros is a valid value. A null new action makes `sigaction` only write
    // the current action to the pointer.
    let action = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        assert_eq!(
            libc::sigaction(signal, std::ptr::null(), &raw mut action),
            0
        );
        action
    };
    match action.sa_sigaction {
        libc::SIG_IGN => "ignore",
        libc::SIG_DFL => "default",
        _ => "handle",
    }
}

#[test]
fn sets_the_dispositions_of_the_erts_emulator_start() {
    // ERTS ignores SIGFPE (`erts_sys_init_float`, sys_float.c:31-37),
    // installs `request_break` for SIGINT and its generic handler for
    // SIGQUIT (`init_break_handler`, sys.c:799-803), its generic handler
    // for SIGTERM and SIGUSR1, and ignores SIGCHLD
    // (`erts_sys_unix_later_init`, sys.c:812-823). The `pbeam` binary calls
    // `install_start_dispositions` before it loads the BEAM files.
    let directory = env::temp_dir().join(format!("pbeam-start-signal-{}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let result = directory.join("result");
    let child = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "isolated_start_dispositions",
            "--include-ignored",
            "--quiet",
        ])
        .env(RESULT, &result)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let written = fs::read_to_string(&result);
    fs::remove_dir_all(&directory).unwrap();
    assert!(child.status.success(), "child exited with {}", child.status);
    assert_eq!(
        written.unwrap(),
        "sigfpe=ignore sigint=handle sigquit=handle sigterm=handle sigusr1=handle sigchld=ignore"
    );
}

/// The child side of `sets_the_dispositions_of_the_erts_emulator_start`:
/// write the dispositions of the start signals after
/// `install_start_dispositions` to the result file. The exec of the child
/// resets each handler, so a handler comes only from the install.
#[test]
#[ignore = "sets_the_dispositions_of_the_erts_emulator_start runs it in a child process"]
fn isolated_start_dispositions() {
    let signals = [
        ("sigfpe", libc::SIGFPE),
        ("sigint", libc::SIGINT),
        ("sigquit", libc::SIGQUIT),
        ("sigterm", libc::SIGTERM),
        ("sigusr1", libc::SIGUSR1),
        ("sigchld", libc::SIGCHLD),
    ];
    install_start_dispositions();
    let dispositions: Vec<String> = signals
        .iter()
        .map(|(name, signal)| format!("{name}={}", disposition(*signal)))
        .collect();
    fs::write(env::var_os(RESULT).unwrap(), dispositions.join(" ")).unwrap();
}

#[test]
fn keeps_erts_alive_after_sighup_in_the_ignore_and_handle_modes() {
    for (mode, signal) in [
        ("ignore", None),
        ("default", Some(libc::SIGHUP)),
        ("handle", None),
    ] {
        let status = Command::new("erl")
            .args([
                "-noinput",
                "-eval",
                &format!(
                    "ok = os:set_signal(sighup, {mode}), \
                     os:cmd(\"kill -HUP \" ++ os:getpid()), timer:sleep(300), halt()."
                ),
            ])
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert_eq!(
            (status.code(), status.signal()),
            (signal.is_none().then_some(0), signal),
            "{mode}"
        );
    }
}
