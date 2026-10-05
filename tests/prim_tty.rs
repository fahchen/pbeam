use std::{
    env, fs,
    os::fd::{FromRawFd, OwnedFd},
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::pinned_beam;

const PRIM_TTY_PROBE: &[u8] = include_bytes!("../fixtures/prim_tty/prim_tty_probe.beam");

/// The stand-in `prim_tty` with the NIF stubs of the pinned module. It is in
/// a subdirectory, so `-pa fixtures/prim_tty` does not load it at boot.
const STAND_IN: &str = "fixtures/prim_tty/nif/prim_tty.beam";
const TTY_NIF_PROBE: &str = "fixtures/prim_tty/tty_nif_probe.beam";

/// The environment variables that give the `isolated_run` child its entry
/// `module:function`, its BEAM files, and the file for its result. The ERTS
/// reference writes its result to the same file variable.
const ENTRY: &str = "PBEAM_TTY_ENTRY";
const BEAMS: &str = "PBEAM_TTY_BEAMS";
const RESULT: &str = "PBEAM_TTY_RESULT";

/// One standard stream of a child process.
#[derive(Clone, Copy)]
enum Stream {
    /// `/dev/null`.
    Null,
    /// A new empty regular file.
    File,
    /// The slave of a new pseudo-terminal with this window size.
    Terminal { columns: u16, rows: u16 },
}

/// Layout F: stdin `/dev/null`, stdout and stderr files.
const FILES: [Stream; 3] = [Stream::Null, Stream::File, Stream::File];
/// Layout P1: stdin `/dev/null`, stdout a 100 by 30 pseudo-terminal, and
/// stderr an 80 by 0 pseudo-terminal.
const OUTPUT_TERMINALS: [Stream; 3] = [
    Stream::Null,
    Stream::Terminal {
        columns: 100,
        rows: 30,
    },
    Stream::Terminal {
        columns: 80,
        rows: 0,
    },
];
/// Layout P2: stdin 120 by 40, stdout 100 by 30, and stderr 0 by 0, all
/// pseudo-terminals.
const TERMINALS: [Stream; 3] = [
    Stream::Terminal {
        columns: 120,
        rows: 40,
    },
    Stream::Terminal {
        columns: 100,
        rows: 30,
    },
    Stream::Terminal {
        columns: 0,
        rows: 0,
    },
];

#[test]
fn raises_badarg_from_the_write_nif_binding_for_a_term_that_is_not_a_tty() {
    // `main/0` starts three pinned `prim_tty:writer_loop/2` writers and
    // returns their exit reasons. The first gets the sync `{write, From,
    // Chars}` message (prim_tty.erl:560), the second gets the async
    // `{write, Chars}` message (prim_tty.erl:554), and the third gets the
    // atom `stdout` as its TTY and `not_iodata` as its iovec. Each writer
    // calls the local `write_nif/2`. The ERTS NIF raises `badarg` because
    // its first argument is not a TTY resource (prim_tty_nif.c:363-364),
    // before it reads the iovec. The logger level `none` stops the
    // asynchronous "Error in process" reports; it does not change the
    // result.
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/prim_tty",
            "-eval",
            "ok = logger:set_primary_config(level, none), \
             io:format(\"~w~n\", [[{R, M, F} || {R, [{M, F, _, _} | _]} <- prim_tty_probe:main()]]), \
             halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"[{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif}]\n"
    );

    // The `nif_start` instruction of `write_nif/2` runs the PBeam binding,
    // which raises `badarg` for each term that is not a TTY resource, also a
    // `make_ref/0` reference. Without the binding, the stub
    // body gives `{undef, []}`. PBeam gives the stack trace `[]`; this is
    // the "Exception stack trace" row in `docs/erts-differences.md`.
    let modules = ["erlang", "prim_tty"].map(pinned_beam);
    let mut beams = vec![PRIM_TTY_PROBE];
    beams.extend(modules.iter().map(Vec::as_slice));
    let program = BeamProgram::load(&beams).unwrap();
    let badarg = Term::Tuple(vec![
        Term::Atom(program.atom("badarg").unwrap()),
        Term::List(vec![]),
    ]);
    assert_eq!(
        program.execute("prim_tty_probe", "main", &[]).unwrap(),
        Term::List(vec![badarg.clone(), badarg.clone(), badarg])
    );
}

#[test]
fn classifies_the_standard_fds_with_isatty() {
    // `tty_nif_probe:isatty/0` calls `prim_tty:isatty/1` with `stdin`,
    // `stdout`, `stderr`, `foo`, `1`, and a reference. The atoms give fds 0,
    // 1, and 2 (prim_tty_nif.c:228-239): a terminal gives `true`, and
    // `ENOTTY` from a file or `/dev/null` gives `false`. Each other term
    // raises `badarg` (prim_tty_nif.c:284).
    for (layout, erts, pbeam) in [
        (
            FILES,
            "[false,false,false,badarg,badarg,badarg]\n",
            r#"Ok(List([Atom("false"), Atom("false"), Atom("false"), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
        (
            OUTPUT_TERMINALS,
            "[false,true,true,badarg,badarg,badarg]\n",
            r#"Ok(List([Atom("false"), Atom("true"), Atom("true"), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
        (
            TERMINALS,
            "[true,true,true,badarg,badarg,badarg]\n",
            r#"Ok(List([Atom("true"), Atom("true"), Atom("true"), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
    ] {
        assert_eq!(run_erts_nif_probe(layout, "isatty"), erts.as_bytes());
        assert_eq!(
            run_isolated(layout, "tty_nif_probe:isatty", &[TTY_NIF_PROBE, STAND_IN]),
            pbeam.as_bytes()
        );
    }
}

#[test]
fn makes_tty_resources_and_reads_the_window_size() {
    // `tty_nif_probe:create/0` makes TTY resources for `stdout` and
    // `stderr`, and returns the `badarg` raises of `stdin` (fd 0), `foo`,
    // `1`, and a reference (prim_tty_nif.c:1003-1004). `window_size/0`
    // reads the window size of the `stdout` and `stderr` resources: the
    // `TIOCGWINSZ` size of fd 0, else of the output fd, where a count of 0
    // gives -1, and `{error, enotsup}` when both counts are -1
    // (prim_tty_nif.c:1179-1189, 1201-1202). An atom, a `make_ref/0`
    // reference, and the reference of a `re_pattern` are not TTY resources,
    // so they raise `badarg` (prim_tty_nif.c:1174-1175).
    let create = "[badarg,badarg,badarg,badarg]\n";
    let create_pbeam =
        r#"Ok(List([Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#;
    for (layout, erts, pbeam) in [
        (
            FILES,
            "[{error,enotsup},{error,enotsup},badarg,badarg,badarg]\n",
            r#"Ok(List([Tuple([Atom("error"), Atom("enotsup")]), Tuple([Atom("error"), Atom("enotsup")]), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
        // fd 0 is `/dev/null`, so the output fd gives the size.
        (
            OUTPUT_TERMINALS,
            "[{ok,{100,30}},{ok,{80,-1}},badarg,badarg,badarg]\n",
            r#"Ok(List([Tuple([Atom("ok"), Tuple([Integer(100), Integer(30)])]), Tuple([Atom("ok"), Tuple([Integer(80), Integer(-1)])]), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
        // fd 0 is a terminal, so it gives the size of both resources.
        (
            TERMINALS,
            "[{ok,{120,40}},{ok,{120,40}},badarg,badarg,badarg]\n",
            r#"Ok(List([Tuple([Atom("ok"), Tuple([Integer(120), Integer(40)])]), Tuple([Atom("ok"), Tuple([Integer(120), Integer(40)])]), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
    ] {
        assert_eq!(run_erts_nif_probe(layout, "create"), create.as_bytes());
        assert_eq!(
            run_isolated(layout, "tty_nif_probe:create", &[TTY_NIF_PROBE, STAND_IN]),
            create_pbeam.as_bytes()
        );
        assert_eq!(run_erts_nif_probe(layout, "window_size"), erts.as_bytes());
        assert_eq!(
            run_isolated(
                layout,
                "tty_nif_probe:window_size",
                &[TTY_NIF_PROBE, STAND_IN]
            ),
            pbeam.as_bytes()
        );
    }
}

#[test]
fn returns_a_runtime_error_from_write_nif_for_a_tty_resource() {
    // `tty_nif_probe:write/0` makes a `stdout` TTY resource and calls
    // `prim_tty:write_nif/2` with it. ERTS writes the iovec to the output
    // fd (prim_tty_nif.c:366-427). PBeam has no host write yet, so the
    // binding returns a runtime error. `tty_create/1` makes no host call,
    // so the test runs in this process.
    let beams = [TTY_NIF_PROBE, STAND_IN].map(|path| fs::read(path).unwrap());
    let program = BeamProgram::load(&beams.each_ref().map(Vec::as_slice)).unwrap();
    let error = program.execute("tty_nif_probe", "write", &[]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "prim_tty:write_nif/2 host write is not implemented yet (#116)"
    );
}

/// Run `tty_nif_probe:Function()` on the pinned ERTS with the stand-in
/// `prim_tty` and the standard streams of `layout`, and return the `~w`
/// text of its result. The probe writes with `file:write_file/2`, because
/// the boot IO servers call the stand-in after `code:load_abs/1`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(run_erts_nif_probe(FILES, "isatty"), b"[false,false,false,badarg,badarg,badarg]\n");
/// ```
fn run_erts_nif_probe(layout: [Stream; 3], function: &str) -> Vec<u8> {
    let mut command = Command::new("erl");
    command.args([
        "-noshell",
        "-nostick",
        "-pa",
        "fixtures/prim_tty",
        "-eval",
        &format!(
            "{{module, prim_tty}} = code:load_abs(\"fixtures/prim_tty/nif/prim_tty\"), \
             ok = prim_tty:load(\"prim_tty\"), \
             ok = file:write_file(os:getenv(\"{RESULT}\"), \
             io_lib:format(\"~w~n\", [tty_nif_probe:{function}()])), halt()."
        ),
    ]);
    run_in_layout(&mut command, layout)
}

/// Run `entry` (`module:function`) of the BEAM files at `beams` in PBeam, in
/// a child copy of this test binary with the standard streams of `layout`,
/// and return the `Debug` text of the result. A child process is necessary,
/// because the `prim_tty` NIF results depend on fds 0, 1, and 2. The child
/// inherits `LLVM_PROFILE_FILE`, so `cargo llvm-cov` measures it.
///
/// # Examples
///
/// ```rust,ignore
/// let result = run_isolated(FILES, "tty_nif_probe:isatty", &[TTY_NIF_PROBE, STAND_IN]);
/// assert!(result.starts_with(b"Ok(List(["));
/// ```
fn run_isolated(layout: [Stream; 3], entry: &str, beams: &[&str]) -> Vec<u8> {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--exact", "isolated_run", "--include-ignored", "--quiet"])
        .env(ENTRY, entry)
        .env(BEAMS, env::join_paths(beams).unwrap());
    run_in_layout(&mut command, layout)
}

/// The child side of `run_isolated`: load the BEAM files, run the entry, and
/// write the `Debug` text of the result to the result file.
#[test]
#[ignore = "run_isolated runs it in a child process with fixed standard streams"]
fn isolated_run() {
    let beams: Vec<Vec<u8>> = env::split_paths(&env::var_os(BEAMS).unwrap())
        .map(|path| fs::read(path).unwrap())
        .collect();
    let beams: Vec<&[u8]> = beams.iter().map(Vec::as_slice).collect();
    let program = BeamProgram::load(&beams).unwrap();
    let entry = env::var(ENTRY).unwrap();
    let (module, function) = entry.split_once(':').unwrap();
    let result = program.execute(module, function, &[]);
    fs::write(env::var_os(RESULT).unwrap(), format!("{result:?}")).unwrap();
}

/// Run `command` with `TERM=dumb`, `LC_ALL=C.UTF-8`, and the standard
/// streams of `layout`, and return the bytes that it writes to the file in
/// the `PBEAM_TTY_RESULT` variable. Each pseudo-terminal master stays open
/// until the command exits.
///
/// # Examples
///
/// ```rust,ignore
/// let result = run_in_layout(Command::new("erl").args(["-noshell", "-eval", "..."]), FILES);
/// ```
fn run_in_layout(command: &mut Command, layout: [Stream; 3]) -> Vec<u8> {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let directory = env::temp_dir().join(format!(
        "pbeam-prim-tty-{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let mut masters = Vec::new();
    let [stdin, stdout, stderr] = std::array::from_fn(|fd| {
        open_stream(layout[fd], &directory.join(format!("fd{fd}")), &mut masters)
    });
    let result = directory.join("result");
    let status = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .env(RESULT, &result)
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .unwrap();
    drop(masters);
    let output = fs::read(&result);
    fs::remove_dir_all(&directory).unwrap();
    assert!(status.success(), "{command:?} exited with {status}");
    output.unwrap()
}

/// Open one standard stream for a child. A file stream creates `path`. A
/// terminal stream adds the master of its pseudo-terminal to `masters`.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = open_stream(Stream::Terminal { columns: 100, rows: 30 }, &path, &mut masters);
/// assert_eq!(masters.len(), 1);
/// ```
fn open_stream(stream: Stream, path: &Path, masters: &mut Vec<OwnedFd>) -> Stdio {
    match stream {
        Stream::Null => Stdio::null(),
        Stream::File => fs::File::create(path).unwrap().into(),
        Stream::Terminal { columns, rows } => {
            let (master, slave) = open_terminal(columns, rows);
            masters.push(master);
            slave.into()
        }
    }
}

/// Open a pseudo-terminal with the window size `columns` by `rows`, and
/// return its master and slave. Both are close-on-exec, so a child of a
/// parallel test does not keep them open.
///
/// # Examples
///
/// ```rust,ignore
/// let (master, slave) = open_terminal(100, 30);
/// ```
fn open_terminal(columns: u16, rows: u16) -> (OwnedFd, OwnedFd) {
    let size = libc::winsize {
        ws_row: rows,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let (mut master, mut slave) = (-1, -1);
    // SAFETY: `openpty` writes two new fds to `master` and `slave`, reads
    // the window size from `size`, and accepts a null name and mode.
    // `fcntl` changes only the fd flags. Each `OwnedFd` takes one new fd.
    unsafe {
        let result = libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw const size as *mut libc::winsize,
        );
        assert_eq!(result, 0, "openpty failed");
        for fd in [master, slave] {
            assert_eq!(libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC), 0);
        }
        (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
    }
}
