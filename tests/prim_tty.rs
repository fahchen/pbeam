use std::{
    env,
    ffi::OsStr,
    fs,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use pbeam_runtime::{BeamProgram, Term};

#[path = "common/boot.rs"]
mod boot;
mod common;

use boot::{BOOT_BEAM, BOOT_MODULES};
use common::{pinned_beam, pinned_beam_path};

const PRIM_TTY_PROBE: &[u8] = include_bytes!("../fixtures/prim_tty/prim_tty_probe.beam");

/// The stand-in `prim_tty` with the NIF stubs of the pinned module. It is in
/// a subdirectory, so `-pa fixtures/prim_tty` does not load it at boot.
const STAND_IN: &str = "fixtures/prim_tty/nif/prim_tty.beam";
const TTY_NIF_PROBE: &str = "fixtures/prim_tty/tty_nif_probe.beam";
const TTY_INIT_PROBE: &str = "fixtures/prim_tty/tty_init_probe.beam";
const LOAD_NIF_PROBE: &str = "fixtures/prim_tty/load_nif_probe.beam";
const COOKED_PROBE: &str = "fixtures/prim_tty/cooked_probe.beam";
const HOST_WRITE_PROBE: &str = "fixtures/prim_tty/host_write_probe.beam";

/// The pinned modules that `prim_tty:init/1` calls in the `disabled` input
/// mode.
const INIT_MODULES: [&str; 11] = [
    "erlang",
    "prim_tty",
    "proc_lib",
    "application",
    "application_controller",
    "ets",
    "re",
    "lists",
    "maps",
    "os",
    "unicode",
];

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
    /// A `Terminal` with `ECHO` and `ICANON` off, as `stty -echo -icanon`
    /// sets it.
    QuietTerminal { columns: u16, rows: u16 },
    /// The write end of a pipe whose read end is closed before the spawn.
    BrokenPipe,
    /// A closed fd.
    Closed,
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

/// Layout T: stdin a 120 by 40 and stdout a 100 by 30 pseudo-terminal, and
/// stderr a file.
const STDIN_STDOUT_TERMINALS: [Stream; 3] = [
    Stream::Terminal {
        columns: 120,
        rows: 40,
    },
    Stream::Terminal {
        columns: 100,
        rows: 30,
    },
    Stream::File,
];

/// Layout M: stdin a 120 by 40 pseudo-terminal with `ECHO` and `ICANON`
/// off, stdout a file, and stderr an 80 by 24 pseudo-terminal. Stdout is
/// not a terminal, so ERTS does not set fd 0 to its start mode when it
/// exits (sys.c:182-190, erl_child_setup.c:563-576).
const MONITOR: [Stream; 3] = [
    Stream::QuietTerminal {
        columns: 120,
        rows: 40,
    },
    Stream::File,
    Stream::Terminal {
        columns: 80,
        rows: 24,
    },
];

#[test]
fn raises_badarg_from_the_write_nif_binding_for_a_term_that_is_not_a_tty() {
    // `main/0` starts four pinned `prim_tty:writer_loop/2` writers and
    // returns their exit reasons. The first gets the sync `{write, From,
    // Chars}` message (prim_tty.erl:560), the second gets the async
    // `{write, Chars}` message (prim_tty.erl:554), the third gets the atom
    // `stdout` as its TTY and `not_iodata` as its iovec, and the fourth
    // gets the reference of a `re_pattern` as its TTY. Each writer
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
        b"[{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif},{badarg,prim_tty,write_nif}]\n"
    );

    // The `nif_start` instruction of `write_nif/2` runs the PBeam binding,
    // which raises `badarg` for each term that is not a TTY resource, also a
    // `make_ref/0` reference and a `re_pattern` reference. Without the binding, the stub
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
        Term::List(vec![badarg.clone(), badarg.clone(), badarg.clone(), badarg])
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
        assert_eq!(run_erts_nif_probe(layout, &[], "isatty"), erts.as_bytes());
        assert_eq!(
            run_isolated(
                layout,
                &[],
                "tty_nif_probe:isatty",
                &[TTY_NIF_PROBE, STAND_IN]
            ),
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
        assert_eq!(run_erts_nif_probe(layout, &[], "create"), create.as_bytes());
        assert_eq!(
            run_isolated(
                layout,
                &[],
                "tty_nif_probe:create",
                &[TTY_NIF_PROBE, STAND_IN]
            ),
            create_pbeam.as_bytes()
        );
        assert_eq!(
            run_erts_nif_probe(layout, &[], "window_size"),
            erts.as_bytes()
        );
        assert_eq!(
            run_isolated(
                layout,
                &[],
                "tty_nif_probe:window_size",
                &[TTY_NIF_PROBE, STAND_IN]
            ),
            pbeam.as_bytes()
        );
    }
}

#[test]
fn reads_the_utf8_codeset_of_the_locale_with_setlocale() {
    // `tty_nif_probe:setlocale/0` calls `prim_tty:setlocale/1` with `foo`
    // and a reference. The NIF does not read its argument. It returns `true`
    // when `setlocale(LC_CTYPE, "")` succeeds and the codeset is `UTF-8`,
    // else `false` (prim_tty_nif.c:766-771).
    for (locale, erts, pbeam) in [
        (
            "C.UTF-8",
            "[true,true]\n",
            r#"Ok(List([Atom("true"), Atom("true")]))"#,
        ),
        (
            "C",
            "[false,false]\n",
            r#"Ok(List([Atom("false"), Atom("false")]))"#,
        ),
    ] {
        let environment = [("LC_ALL", locale)];
        assert_eq!(
            run_erts_nif_probe(FILES, &environment, "setlocale"),
            erts.as_bytes()
        );
        assert_eq!(
            run_isolated(
                FILES,
                &environment,
                "tty_nif_probe:setlocale",
                &[TTY_NIF_PROBE, STAND_IN]
            ),
            pbeam.as_bytes()
        );
    }
}

#[test]
fn reads_the_terminfo_entry_with_setupterm() {
    // `tty_nif_probe:setupterm/0` calls `prim_tty:setupterm_nif/0` two
    // times, so the second success frees the terminal of the first with
    // `del_curterm` (prim_tty_nif.c:785-788).
    assert_eq!(run_erts_nif_probe(FILES, &[], "setupterm"), b"[ok,ok]\n");
    assert_eq!(
        run_isolated(
            FILES,
            &[],
            "tty_nif_probe:setupterm",
            &[TTY_NIF_PROBE, STAND_IN]
        ),
        br#"Ok(List([Atom("ok"), Atom("ok")]))"#
    );

    // A `TERM` without a terminfo entry makes `setupterm` fail, and the
    // result is `{error, {setupterm, Errno}}` with the `erl_errno_id` atom
    // of the `errno` that `setupterm` leaves (prim_tty_nif.c:214-226,
    // 780-782). The curses library sets this `errno`, so the test takes the
    // atom from the ERTS run on the same host.
    let environment = [("TERM", "pbeam-unknown-term")];
    let erts = run_erts_nif_probe(FILES, &environment, "setupterm");
    #[cfg(target_os = "macos")]
    assert_eq!(
        erts,
        b"[{error,{setupterm,enoent}},{error,{setupterm,enoent}}]\n"
    );
    let errno = String::from_utf8(erts)
        .unwrap()
        .strip_prefix("[{error,{setupterm,")
        .and_then(|rest| rest.split_once('}'))
        .unwrap()
        .0
        .to_owned();
    let error = format!(r#"Tuple([Atom("error"), Tuple([Atom("setupterm"), Atom("{errno}")])])"#);
    assert_eq!(
        String::from_utf8(run_isolated(
            FILES,
            &environment,
            "tty_nif_probe:setupterm",
            &[TTY_NIF_PROBE, STAND_IN]
        ))
        .unwrap(),
        format!("Ok(List([{error}, {error}]))")
    );
}

#[test]
fn sets_the_terminal_mode_with_tty_init() {
    // `tty_nif_probe:init/0` calls `prim_tty:tty_init/2`. A term that is not
    // a map, three terms that are not a TTY resource (an atom, a
    // `make_ref/0` reference, and a `re_pattern` reference), and a map
    // without `input` raise `badarg` in this order (prim_tty_nif.c:1081-1089). On the
    // `stdout` resource, `raw` sets the raw mode, `cooked` sets the first
    // mode again, and `foo` finds the first mode set and does not call
    // `tcsetattr` (prim_tty_nif.c:1099-1134). Last, `raw` on the `stderr`
    // resource. A resource on a file is `unavailable`: `raw` gives
    // `{error, enotsup}` and each other input gives `ok`
    // (prim_tty_nif.c:1091-1095).
    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("init"), FILES, &[]);
    assert_eq!(
        erts,
        b"[badarg,badarg,badarg,badarg,badarg,{error,enotsup},ok,ok,{error,enotsup}]\n"
    );
    let probe = || isolated("tty_nif_probe:init", &[TTY_NIF_PROBE, STAND_IN]);
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), FILES, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(List([Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Tuple([Atom("error"), Atom("enotsup")]), Atom("ok"), Atom("ok"), Tuple([Atom("error"), Atom("enotsup")])]))"#
    );
    assert_same_modes(&erts_modes, &pbeam_modes);

    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("init"), OUTPUT_TERMINALS, &[]);
    assert_eq!(erts, b"[badarg,badarg,badarg,badarg,badarg,ok,ok,ok,ok]\n");
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), OUTPUT_TERMINALS, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(List([Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("ok"), Atom("ok"), Atom("ok"), Atom("ok")]))"#
    );
    // The stdout terminal has its first mode again, and the stderr terminal
    // has the raw mode.
    assert_same_modes(&erts_modes, &pbeam_modes);
    let [_, Some(stdout), Some(stderr)] = pbeam_modes else {
        panic!("stdout and stderr are terminals");
    };
    assert_eq!(stdout.c_lflag & CANONICAL_ECHO, CANONICAL_ECHO);
    assert_raw(&stderr);
}

#[test]
fn restores_fd_0_when_the_tty_owner_exits() {
    // `tty_nif_probe:monitor/0` spawns `owner/1` and monitors it. The owner
    // makes a `stderr` resource, and a first holder process sets the raw
    // mode on it. The owner reads its window size. It then makes a second
    // and a third `stderr` resource, whose first mode is that raw mode, and
    // calls `tty_init/2` on the second, the first, and the third resource.
    // Each successful `tty_init/2` makes the caller the `self` pid of the
    // resource and monitors it (prim_tty_nif.c:1162-1165). A second holder
    // then calls `tty_init/2` on the third resource and becomes its `self`
    // pid. When the owner exits, each monitor of a resource whose `self`
    // pid is still the owner sets fd 0 to the first mode of the resource
    // (prim_tty_nif.c:1235-1241). In the order of the calls, the monitor of
    // the second resource sets the raw mode, the monitor of the first
    // resource then sets the first mode of the stderr terminal, and the
    // monitor of the third resource does nothing, because the second
    // holder owns it. The reverse order, or a restore without the owner
    // check, leaves the raw mode on fd 0.
    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("monitor"), FILES, &[]);
    assert_eq!(
        erts,
        b"{[{error,enotsup},{error,enotsup},ok,{error,enotsup},ok,ok],normal}\n"
    );
    let probe = || isolated("tty_nif_probe:monitor", &[TTY_NIF_PROBE, STAND_IN]);
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), FILES, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(Tuple([List([Tuple([Atom("error"), Atom("enotsup")]), Tuple([Atom("error"), Atom("enotsup")]), Atom("ok"), Tuple([Atom("error"), Atom("enotsup")]), Atom("ok"), Atom("ok")]), Atom("normal")]))"#
    );
    assert_same_modes(&erts_modes, &pbeam_modes);

    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("monitor"), MONITOR, &[]);
    assert_eq!(erts, b"{[ok,{ok,{120,40}},ok,ok,ok,ok],normal}\n");
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), MONITOR, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(Tuple([List([Atom("ok"), Tuple([Atom("ok"), Tuple([Integer(120), Integer(40)])]), Atom("ok"), Atom("ok"), Atom("ok"), Atom("ok")]), Atom("normal")]))"#
    );
    assert_same_modes(&erts_modes, &pbeam_modes);
    let [Some(stdin), None, Some(stderr)] = pbeam_modes else {
        panic!("stdin and stderr are terminals");
    };
    // Stdin started without `ECHO` and `ICANON`, so these flags at exit
    // show that a TTY monitor set fd 0 to the first mode of stderr.
    assert_eq!(stdin.c_lflag & CANONICAL_ECHO, CANONICAL_ECHO);
    assert_raw(&stderr);
}

#[test]
fn selects_stdin_and_reads_the_encoding() {
    // `tty_nif_probe:select/0` calls `prim_tty:tty_select/2` on a `stdout`
    // resource with a `make_ref/0` reference, `foo`, and `undefined` as the
    // select reference: each gives `ok`. An atom, a `make_ref/0` reference,
    // and a `re_pattern` reference are not TTY resources, so they raise
    // `badarg` (prim_tty_nif.c:1220-1221). On ERTS the caller also gets one
    // `{select, Out, undefined, ready_input}` message, which the probe does
    // not read; PBeam has no IO poll ("Input select" in
    // docs/erts-differences.md). `encoding/0` calls
    // `prim_tty:tty_encoding/1` with a resource, an atom, an integer, and a
    // reference: Unix gives `utf8` for each term (prim_tty_nif.c:287-297).
    for (function, erts, pbeam) in [
        (
            "select",
            "[ok,ok,ok,badarg,badarg,badarg]\n",
            r#"Ok(List([Atom("ok"), Atom("ok"), Atom("ok"), Atom("badarg"), Atom("badarg"), Atom("badarg")]))"#,
        ),
        (
            "encoding",
            "[utf8,utf8,utf8,utf8]\n",
            r#"Ok(List([Atom("utf8"), Atom("utf8"), Atom("utf8"), Atom("utf8")]))"#,
        ),
    ] {
        assert_eq!(run_erts_nif_probe(FILES, &[], function), erts.as_bytes());
        assert_eq!(
            run_isolated(
                FILES,
                &[],
                &format!("tty_nif_probe:{function}"),
                &[TTY_NIF_PROBE, STAND_IN]
            ),
            pbeam.as_bytes()
        );
    }
}

#[test]
fn restores_fd_0_from_the_monitor_of_tty_select() {
    // `tty_nif_probe:select_monitor/0` spawns `selector/1` and monitors it.
    // A holder sets the raw mode on a first `stderr` resource. The selector
    // makes a second `stderr` resource, whose first mode is that raw mode,
    // and calls `tty_init/2` on the first and the second resource, then
    // `tty_select/2` on the first. `tty_select/2` monitors the caller again
    // (prim_tty_nif.c:1229-1230). When the selector exits, its monitors set
    // fd 0 to the first mode of the stderr terminal, to the raw mode, and
    // to the first mode again (prim_tty_nif.c:1235-1241). Without the
    // monitor of `tty_select/2`, fd 0 ends in the raw mode.
    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("select_monitor"), MONITOR, &[]);
    assert_eq!(erts, b"{[ok,ok,ok,ok],normal}\n");
    let mut probe = isolated("tty_nif_probe:select_monitor", &[TTY_NIF_PROBE, STAND_IN]);
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe, MONITOR, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(Tuple([List([Atom("ok"), Atom("ok"), Atom("ok"), Atom("ok")]), Atom("normal")]))"#
    );
    assert_same_modes(&erts_modes, &pbeam_modes);
    let [Some(stdin), None, Some(_)] = pbeam_modes else {
        panic!("stdin and stderr are terminals");
    };
    // Stdin started without `ECHO` and `ICANON`.
    assert_eq!(stdin.c_lflag & CANONICAL_ECHO, CANONICAL_ECHO);
}

#[test]
fn restores_fd_0_when_a_linked_tty_owner_exits_abnormally() {
    // `tty_nif_probe:link/0` monitors a linker process. The linker spawns
    // and links an owner. The owner sets the raw mode on a `stderr`
    // resource and exits with the reason `boom`. ERTS runs the TTY monitor
    // of the owner on each exit (prim_tty_nif.c:1235-1241), so fd 0 gets
    // the first mode of the stderr terminal. The linker does not trap
    // exits, so ERTS also ends it with `boom`. PBeam does not propagate
    // this exit signal yet and returns a runtime error, but it must
    // restore fd 0 before it returns the error.
    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("link"), MONITOR, &[]);
    assert_eq!(erts, b"{ok,boom}\n");
    let probe = || isolated("tty_nif_probe:link", &[TTY_NIF_PROBE, STAND_IN]);
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), MONITOR, &[]);
    assert_eq!(
        String::from_utf8(pbeam).unwrap(),
        "Err(BeamError(\"unsupported exit signal Atom(\\\"boom\\\") to a process that does not trap exits\"))"
    );
    assert_same_modes(&erts_modes, &pbeam_modes);
    let [Some(stdin), None, Some(stderr)] = pbeam_modes else {
        panic!("stdin and stderr are terminals");
    };
    // Stdin started without `ECHO` and `ICANON`, so these flags at exit
    // show that a TTY monitor set fd 0 to the first mode of stderr.
    assert_eq!(stdin.c_lflag & CANONICAL_ECHO, CANONICAL_ECHO);
    assert_raw(&stderr);
}

#[test]
fn writes_the_iovec_of_a_tty_resource_with_writev() {
    // `tty_nif_probe:write/0` calls `prim_tty:write_nif/2` with a `stderr`
    // TTY resource and ten terms. The NIF reads the iovec in groups of 1024
    // elements and writes each group with `writev` before it reads the next
    // group (prim_tty_nif.c:366-423). An empty binary gives no `writev`
    // entry (erl_nif.c:4122-4135). A term that is not a list, or a group
    // with an element that is not a binary, raises `badarg`
    // (erl_nif.c:4113-4115, 4145-4147). So `Group ++ [foo]` (one group)
    // writes nothing, and `[<<"c">> | Group] ++ [foo]` writes its first
    // group of 1024 binaries. `[<<>>]` calls `writev` with a count of 0,
    // which fails with `EINVAL` on macOS and writes 0 bytes on Linux.
    let last = if cfg!(target_os = "macos") {
        (
            "{error,einval}",
            r#"Tuple([Atom("error"), Atom("einval")])"#,
        )
    } else {
        ("ok", r#"Atom("ok")"#)
    };
    let group = format!("c{}", "b".repeat(1023));
    let stderr = format!("Hello, world!\n{group}{group}d");
    assert_eq!(stderr.len(), 2063);

    let outputs = run_with_outputs(&mut erts_nif_probe("write"), FILES, &[]);
    assert_eq!(
        outputs.result.as_deref(),
        Some(
            format!(
                "[ok,ok,badarg,badarg,badarg,badarg,badarg,badarg,ok,{}]\n",
                last.0
            )
            .as_bytes()
        )
    );
    assert_eq!(outputs.files[1].as_deref(), Some(&b""[..]));
    assert_eq!(outputs.files[2].as_deref(), Some(stderr.as_bytes()));

    let outputs = run_with_outputs(
        &mut isolated("tty_nif_probe:write", &[TTY_NIF_PROBE, STAND_IN]),
        FILES,
        &[],
    );
    assert_eq!(
        outputs.result.as_deref(),
        Some(
            format!(
                r#"Ok(List([Atom("ok"), Atom("ok"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("ok"), {}]))"#,
                last.1
            )
            .as_bytes()
        )
    );
    assert_eq!(outputs.files[2].as_deref(), Some(stderr.as_bytes()));
}

#[test]
fn returns_the_errno_of_a_failed_tty_write() {
    // `tty_nif_probe:write_error/0` writes to a `stderr` TTY resource, and
    // stderr is a pipe whose reader is closed. ERTS ignores `SIGPIPE`
    // (sys_drivers.c:175), and so does the Rust runtime before `main`, so
    // `writev` fails with `EPIPE`, and the result is `{error, epipe}`
    // (prim_tty_nif.c:403-406). `[]` writes nothing, and `[<<"x">>, foo]`
    // raises `badarg` before the write (prim_tty_nif.c:366-368).
    let last = if cfg!(target_os = "macos") {
        (
            "{error,einval}",
            r#"Tuple([Atom("error"), Atom("einval")])"#,
        )
    } else {
        ("ok", r#"Atom("ok")"#)
    };
    let layout = [Stream::Null, Stream::File, Stream::BrokenPipe];
    assert_eq!(
        run_in_layout(&mut erts_nif_probe("write_error"), layout, &[]),
        format!("[ok,{{error,epipe}},badarg,{}]\n", last.0).as_bytes()
    );
    assert_eq!(
        run_in_layout(
            &mut isolated("tty_nif_probe:write_error", &[TTY_NIF_PROBE, STAND_IN]),
            layout,
            &[]
        ),
        format!(
            r#"Ok(List([Atom("ok"), Tuple([Atom("error"), Atom("epipe")]), Atom("badarg"), {}]))"#,
            last.1
        )
        .as_bytes()
    );
}

#[test]
fn runs_prim_tty_init_in_the_disabled_input_mode() {
    // `tty_init_probe:main/0` returns `isatty/1` of the three standard
    // atoms and the result of `prim_tty:init/1` in three new processes:
    // output `raw` on stdout, output `cooked` on `ofd => stderr`, and
    // `ofd => stdin`. `prim_tty:init/1` calls `tty_create/1`,
    // `setlocale/1`, `setupterm_nif/0`, `tty_init/2`, and
    // `tty_window_size/1`, and starts the writer process
    // (prim_tty.erl:216-306). Each result is `unicode/1`, `output_mode/1`,
    // the read handle, `is_writer/2` of the registered writer, and
    // `window_size/1`. `tty_create(stdin)` raises `badarg`
    // (prim_tty_nif.c:1003-1004). `pbeam_main/0` first makes the `ac_tab`
    // table that `application:get_env/3` reads, because PBeam has no
    // application controller.
    let beams: Vec<_> = INIT_MODULES
        .map(pinned_beam_path)
        .into_iter()
        .chain([PathBuf::from(TTY_INIT_PROBE)])
        .collect();
    let probe = || isolated("tty_init_probe:pbeam_main", &beams);
    let init = |unicode: &str, size: &str| {
        format!(
            r#"List([Atom("{unicode}"), Atom("raw"), Atom("undefined"), Atom("true"), {size}]), List([Atom("{unicode}"), Atom("cooked"), Atom("undefined"), Atom("true"), {size}]), Atom("badarg")"#
        )
    };
    let enotsup = r#"Tuple([Atom("error"), Atom("enotsup")])"#;
    for (locale, erts, unicode) in [
        (
            "C.UTF-8",
            "[false,false,false,[true,raw,undefined,true,{error,enotsup}],[true,cooked,undefined,true,{error,enotsup}],badarg]\n",
            "true",
        ),
        (
            "C",
            "[false,false,false,[false,raw,undefined,true,{error,enotsup}],[false,cooked,undefined,true,{error,enotsup}],badarg]\n",
            "false",
        ),
    ] {
        let environment = [("LC_ALL", locale)];
        let outputs = run_with_outputs(&mut erts_init_probe(), FILES, &environment);
        assert_eq!(outputs.files[1].as_deref(), Some(&b""[..]));
        assert_eq!(outputs.files[2].as_deref(), Some(erts.as_bytes()));
        assert_eq!(
            String::from_utf8(run_in_layout(&mut probe(), FILES, &environment)).unwrap(),
            format!(
                r#"Ok(List([Atom("false"), Atom("false"), Atom("false"), {}]))"#,
                init(unicode, enotsup)
            )
        );
    }

    // Layout T: fd 0 is a terminal, so it gives the window size of each
    // output fd (prim_tty_nif.c:1179-1189). In the `disabled` input mode,
    // `tty_init/2` keeps the first mode of stdout (prim_tty_nif.c:1103).
    // When the `init/3` process of stdout exits, its monitor sets fd 0 to
    // that first mode (prim_tty_nif.c:1235-1241). Both terminals must end
    // in the modes of the ERTS run.
    let outputs = run_with_outputs(&mut erts_init_probe(), STDIN_STDOUT_TERMINALS, &[]);
    assert_eq!(
        outputs.files[2].as_deref(),
        Some(&b"[true,true,false,[true,raw,undefined,true,{ok,{120,40}}],[true,cooked,undefined,true,{ok,{120,40}}],badarg]\n"[..])
    );
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), STDIN_STDOUT_TERMINALS, &[]);
    assert_eq!(
        String::from_utf8(pbeam).unwrap(),
        format!(
            r#"Ok(List([Atom("true"), Atom("true"), Atom("false"), {}]))"#,
            init(
                "true",
                r#"Tuple([Atom("ok"), Tuple([Integer(120), Integer(40)])])"#
            )
        )
    );
    assert_same_modes(&outputs.modes, &pbeam_modes);
}

#[test]
fn writes_hello_world_from_the_pbeam_binary_through_a_prim_tty_writer() {
    // `host_write_probe:main/0` starts a `prim_tty` writer on stdout and a
    // second writer on stderr, each in the `raw` output mode. It sends
    // `"Hello, world!\n"` to the stdout writer with `prim_tty:write/3`, which
    // calls `write_nif/2` and replies `ok` (prim_tty.erl:541-566,
    // prim_tty_nif.c:354-427). It then writes that reply and a newline to
    // stderr. The `pbeam` binary runs the boot module `pbeam_boot`, which
    // starts the standard IO servers first, as `erl -noshell` does, and
    // makes `ac_tab`. With stdout closed, ERTS opens `/dev/null` as fd 1
    // (sys.c:392-399) and ignores `SIGPIPE` (sys_drivers.c:175). The Rust
    // runtime of the `pbeam` binary does the same before `main` (Rust
    // 1.99.0 library/std/src/sys/pal/unix/mod.rs:22-36, 51-139, 141-185), so
    // the write to stdout gives `ok`. With stdout a pipe whose reader is
    // closed, `write_nif/2` gives `{error, epipe}`, and the writer calls
    // `exit(self(), epipe)` (prim_tty.erl:559-566). It does not trap exits,
    // so it exits before `exit/2` returns (bif.c:1759-1772). The probe writes
    // the `DOWN` reason of its monitor and the `EXIT` reason of its link to
    // stderr.
    let beams = BOOT_MODULES.map(pinned_beam_path);
    let pbeam = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_pbeam"));
        command
            .args([
                "pbeam_boot",
                "start",
                "host_write_probe",
                "main",
                "--",
                HOST_WRITE_PROBE,
            ])
            .args(&beams)
            .arg(BOOT_BEAM);
        command
    };
    let commands: [(&str, &dyn Fn() -> Command); 2] =
        [("ERTS", &erts_host_write_probe), ("PBeam", &pbeam)];
    for (name, command) in commands {
        let outputs = run_with_outputs(&mut command(), FILES, &[]);
        assert_eq!(
            outputs.files[1].as_deref(),
            Some(&b"Hello, world!\n"[..]),
            "{name}"
        );
        assert_eq!(outputs.files[2].as_deref(), Some(&b"ok\n"[..]), "{name}");

        let layout = [Stream::Null, Stream::Closed, Stream::File];
        let outputs = run_with_outputs(&mut command(), layout, &[]);
        assert_eq!(outputs.files[2].as_deref(), Some(&b"ok\n"[..]), "{name}");

        let layout = [Stream::Null, Stream::BrokenPipe, Stream::File];
        let outputs = run_with_outputs(&mut command(), layout, &[]);
        assert_eq!(
            outputs.files[2].as_deref(),
            Some(&b"{epipe,epipe}\n"[..]),
            "{name}"
        );
    }
}

#[test]
fn starts_the_cooked_reader_and_the_signal_handler_with_prim_tty_init() {
    // `cooked_probe:main/0` calls `prim_tty:init/1` in the `cooked` input
    // mode from a registered process. It starts the reader process, which
    // calls `tty_select/2` and `tty_encoding/1` (prim_tty.erl:436-448), and
    // adds `prim_tty_sighandler` to `erl_signal_server` with
    // `gen_event:add_handler/3` (prim_tty.erl:284-298). The handler init
    // matches its state with `has_map_fields` and calls `os:set_signal/2`
    // for `sigcont` and `sigwinch` (prim_tty_sighandler.erl:28-31). The probe
    // then sends `{notify, sigwinch}`, the message of the ERTS signal
    // dispatcher (sys.c:589-607), to `erl_signal_server`; the handler
    // forwards it to the `prim_tty:init/1` caller only when it has the
    // correct state (prim_tty_sighandler.erl:33-36). `pbeam_main/0` first
    // makes `ac_tab` and starts `erl_signal_server` with
    // `erl_signal_handler`, as the kernel boot does. The PBeam run is a
    // child, because the handler init changes the signal dispositions.
    let beams: Vec<_> = [
        "erlang",
        "prim_tty",
        "proc_lib",
        "application",
        "application_controller",
        "ets",
        "re",
        "lists",
        "maps",
        "os",
        "unicode",
        "gen_event",
        "gen",
        "prim_tty_sighandler",
        "erl_signal_handler",
    ]
    .map(pinned_beam_path)
    .into_iter()
    .chain([PathBuf::from(COOKED_PROBE)])
    .collect();
    for (locale, unicode) in [("C.UTF-8", "true"), ("C", "false")] {
        let environment = [("LC_ALL", locale)];
        let outputs = run_with_outputs(&mut erts_cooked_probe("main"), FILES, &environment);
        assert_eq!(outputs.files[1].as_deref(), Some(&b""[..]));
        assert_eq!(
            outputs.files[2].as_deref(),
            Some(
                format!("[{unicode},raw,true,true,{{error,enotsup}},sigwinch,[prim_tty_sighandler,erl_signal_handler]]\n")
                    .as_bytes()
            )
        );
        assert_eq!(
            String::from_utf8(run_in_layout(
                &mut isolated("cooked_probe:pbeam_main", &beams),
                FILES,
                &environment
            ))
            .unwrap(),
            format!(
                r#"Ok(List([Atom("{unicode}"), Atom("raw"), Atom("true"), Atom("true"), Tuple([Atom("error"), Atom("enotsup")]), Atom("sigwinch"), List([Atom("prim_tty_sighandler"), Atom("erl_signal_handler")])]))"#
            )
        );
    }

    // `cooked_probe:read/0` calls `prim_tty:read/1` and waits for the data
    // of the reader. On ERTS the reader gets the select message of fd 0 and
    // reads `eof` from `/dev/null` (prim_tty.erl:470-490). PBeam has no IO
    // poll, so the message never arrives and the run ends with a runtime
    // error ("Input select" in docs/erts-differences.md).
    let outputs = run_with_outputs(&mut erts_cooked_probe("read"), FILES, &[]);
    assert_eq!(outputs.files[1].as_deref(), Some(&b""[..]));
    assert_eq!(outputs.files[2].as_deref(), Some(&b"eof\n"[..]));
    assert_eq!(
        String::from_utf8(run_in_layout(
            &mut isolated("cooked_probe:pbeam_read", &beams),
            FILES,
            &[]
        ))
        .unwrap(),
        r#"Err(BeamError("root process waits for a message that cannot arrive"))"#
    );
}

#[test]
fn runs_load_nif_as_an_instruction_with_the_host_filename_encoding() {
    // `load_nif_probe:main/0` calls `erlang:load_nif/2` through the
    // `call_ext_only` (`prim_tty:load/1`), `call_ext`
    // (`prim_tty:load_body/1`), and `call_ext_last` (`prim_tty:load_last/1`)
    // forms of the stand-in. Before the load, each file name that ERTS does
    // not accept raises `badarg` (erl_nif.c:4730-4741, erl_unicode.c:2057-2119,
    // 2222-2398). The first valid call returns `ok`. Then each valid name
    // gives the `reload` error (erl_nif.c:4774-4777), and each name that is
    // not valid still raises `badarg`. A code point above 255 is valid only
    // with the UTF-8 file name encoding: always on macOS, and on another host
    // only when the `LC_CTYPE` codeset is `UTF-8`
    // (erl_sys_common_misc.c:54-56, 95-124).
    let reload = Value::error(
        "reload",
        "NIF library already loaded (reload disallowed since OTP 20).",
    );
    let badarg = || Value::Atom("badarg");
    for locale in ["C.UTF-8", "C", "pbeam-unknown"] {
        let utf8 = cfg!(target_os = "macos") || locale == "C.UTF-8";
        let wide = || if utf8 { reload.clone() } else { badarg() };
        let mut loaded = vec![reload.clone(); 6];
        loaded.extend([wide(), wide(), badarg(), badarg()]);
        let expected = Value::List(vec![
            Value::List(vec![badarg(); 7]),
            Value::Tuple(vec![Value::Atom("ok"), Value::Atom("body")]),
            Value::List(loaded),
            reload.clone(),
            badarg(),
            badarg(),
        ]);
        assert_load_nif_probe(&[("LC_ALL", locale)], "nif", "main", &expected);
    }
}

#[test]
fn checks_the_library_functions_against_the_calling_module() {
    // `load_nif_probe:twice/0` calls `prim_tty:load/1` two times. In the
    // order of `nif_funcs` (prim_tty_nif.c:153-174), a function that the
    // module does not define gives "Function not found", and a function
    // that is not in the `-nifs` list gives "Function not declared as nif"
    // (erl_nif.c:4845-4866). `nif_missing` has no `tty_init/2` and no
    // `tputs_nif/2`, and `nif_undeclared` has no `setlocale/1` in `-nifs`
    // and no `tputs_nif/2`. An error does not load the library, so the
    // second call gives the same error (erl_nif.c:5019-5034). A module
    // without `-nifs` accepts each function (erl_nif.c:4700-4704).
    let loaded = Value::List(vec![
        Value::Atom("ok"),
        Value::error(
            "reload",
            "NIF library already loaded (reload disallowed since OTP 20).",
        ),
    ]);
    let twice = |text| Value::List(vec![Value::error("bad_lib", text); 2]);
    for (stand_in, expected) in [
        ("nif", loaded.clone()),
        ("nif_plain", loaded),
        (
            "nif_missing",
            twice("Function not found prim_tty:tty_init/2"),
        ),
        (
            "nif_undeclared",
            twice("Function not declared as nif prim_tty:setlocale/1"),
        ),
    ] {
        assert_load_nif_probe(&[], stand_in, "twice", &expected);
    }
}

#[test]
fn raises_badarg_for_a_module_without_bindings_before_the_library_check() {
    // `load_nif_probe:other_badarg/0` calls `erlang:load_nif(foo, 0)` from
    // `load_nif_probe`. The file name check comes before the library check
    // (erl_nif.c:4730-4757).
    assert_load_nif_probe(&[], "nif", "other_badarg", &Value::Atom("badarg"));
}

#[test]
fn returns_bad_lib_from_the_bif_for_apply_and_fun_calls() {
    // `load_nif_probe:bad_lib/0` calls `erlang:load_nif/2` through `apply`
    // with a module that is not a literal and through a call of `fun
    // erlang:load_nif/2`. Only the `call_ext` forms become `i_load_nif`
    // (jit/arm/ops.tab:615-623), so these calls run the BIF, which returns
    // the `bad_lib` error and does not check the file name `foo`
    // (erl_nif.c:4618-4626).
    let bad_lib = Value::error(
        "bad_lib",
        "load_nif/2 must be explicitly called from the NIF module. It cannot be called through apply/3.",
    );
    assert_load_nif_probe(&[], "nif", "bad_lib", &Value::List(vec![bad_lib; 3]));
}

#[test]
fn returns_a_runtime_error_for_a_module_without_bindings() {
    // `load_nif_probe:other/0` calls `erlang:load_nif("load_nif_probe", 0)`.
    // ERTS opens the library with `dlopen` and returns `{error,
    // {load_failed, Text}}` with host text (erl_nif.c:4778-4787). PBeam
    // opens no library and returns a runtime error.
    let erts = String::from_utf8(run_in_layout(
        &mut erts_load_nif_probe("nif", "other"),
        FILES,
        &[],
    ))
    .unwrap();
    let slogan = Value::Text("Failed to load NIF library").erts();
    let prefix = format!(
        "{{error,{{load_failed,{}",
        slogan.strip_suffix(']').unwrap()
    );
    assert!(erts.starts_with(&prefix), "{erts}");
    assert_eq!(
        run_isolated(
            FILES,
            &[],
            "load_nif_probe:other",
            &[LOAD_NIF_PROBE, STAND_IN]
        ),
        br#"Err(BeamError("erlang:load_nif/2 from load_nif_probe is not supported: PBeam has no native bindings for the module"))"#
    );
}

/// An expected Erlang term, written as ERTS `~w` text and as the `Debug`
/// text of the PBeam term.
#[derive(Clone)]
enum Value {
    /// An atom.
    Atom(&'static str),
    /// A string: a list of the bytes of the text.
    Text(&'static str),
    /// A tuple.
    Tuple(Vec<Value>),
    /// A list.
    List(Vec<Value>),
}

impl Value {
    /// Return `{error, {Kind, Text}}`, the result of the ERTS
    /// `load_nif_error` (erl_nif.c:4530-4558).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Value::error("reload", "a").erts(), "{error,{reload,[97]}}");
    /// ```
    fn error(kind: &'static str, text: &'static str) -> Value {
        Value::Tuple(vec![
            Value::Atom("error"),
            Value::Tuple(vec![Value::Atom(kind), Value::Text(text)]),
        ])
    }

    /// Return the ERTS `~w` text of the term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Value::List(vec![Value::Atom("ok"), Value::Text("a")]).erts(), "[ok,[97]]");
    /// ```
    fn erts(&self) -> String {
        match self {
            Value::Atom(name) => (*name).to_owned(),
            Value::Text(text) => format!(
                "[{}]",
                text.bytes()
                    .map(|byte| byte.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Value::Tuple(elements) => format!("{{{}}}", Value::join(elements, Value::erts, ",")),
            Value::List(elements) => format!("[{}]", Value::join(elements, Value::erts, ",")),
        }
    }

    /// Return the `Debug` text of the PBeam term.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Value::List(vec![Value::Atom("ok"), Value::Text("a")]).pbeam(), r#"List([Atom("ok"), List([Integer(97)])])"#);
    /// ```
    fn pbeam(&self) -> String {
        match self {
            Value::Atom(name) => format!("Atom({name:?})"),
            Value::Text(text) => format!(
                "List([{}])",
                text.bytes()
                    .map(|byte| format!("Integer({byte})"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Value::Tuple(elements) => {
                format!("Tuple([{}])", Value::join(elements, Value::pbeam, ", "))
            }
            Value::List(elements) => {
                format!("List([{}])", Value::join(elements, Value::pbeam, ", "))
            }
        }
    }

    /// Join the text of each element with `separator`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Value::join(&[Value::Atom("a"), Value::Atom("b")], Value::erts, ","), "a,b");
    /// ```
    fn join(elements: &[Value], text: fn(&Value) -> String, separator: &str) -> String {
        elements
            .iter()
            .map(text)
            .collect::<Vec<_>>()
            .join(separator)
    }
}

/// Run `load_nif_probe:Function()` with the stand-in `prim_tty` of
/// `fixtures/prim_tty/<stand_in>` and the variables of `environment`, on
/// the pinned ERTS and in PBeam, and assert that both give `expected`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_load_nif_probe(&[], "nif", "other_badarg", &Value::Atom("badarg"));
/// ```
fn assert_load_nif_probe(
    environment: &[(&str, &str)],
    stand_in: &str,
    function: &str,
    expected: &Value,
) {
    assert_eq!(
        String::from_utf8(run_in_layout(
            &mut erts_load_nif_probe(stand_in, function),
            FILES,
            environment
        ))
        .unwrap(),
        format!("{}\n", expected.erts()),
        "ERTS {stand_in} {function} {environment:?}"
    );
    let stand_in = format!("fixtures/prim_tty/{stand_in}/prim_tty.beam");
    assert_eq!(
        String::from_utf8(run_isolated(
            FILES,
            environment,
            &format!("load_nif_probe:{function}"),
            &[LOAD_NIF_PROBE, &stand_in]
        ))
        .unwrap(),
        format!("Ok({})", expected.pbeam()),
        "PBeam {stand_in} {function} {environment:?}"
    );
}

/// Return the `erl` command that writes the `~w` text of
/// `load_nif_probe:Function()` to the result file, with the stand-in
/// `prim_tty` of `fixtures/prim_tty/<stand_in>`. It does not call
/// `prim_tty:load/1` first.
///
/// # Examples
///
/// ```rust,ignore
/// let result = run_in_layout(&mut erts_load_nif_probe("nif", "main"), FILES, &[]);
/// ```
fn erts_load_nif_probe(stand_in: &str, function: &str) -> Command {
    erts_stand_in_probe(stand_in, &format!("load_nif_probe:{function}()"))
}

/// Return the `erl` command that loads the stand-in `prim_tty` of
/// `fixtures/prim_tty/<stand_in>` and writes the `~w` text of the Erlang
/// expression `expression` to the result file.
///
/// # Examples
///
/// ```rust,ignore
/// let result = run_in_layout(&mut erts_stand_in_probe("nif", "load_nif_probe:main()"), FILES, &[]);
/// ```
fn erts_stand_in_probe(stand_in: &str, expression: &str) -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noinput",
        "-nostick",
        "-pa",
        "fixtures/prim_tty",
        "-eval",
        &format!(
            "{{module, prim_tty}} = code:load_abs(\"fixtures/prim_tty/{stand_in}/prim_tty\"), \
             ok = file:write_file(os:getenv(\"{RESULT}\"), \
             io_lib:format(\"~w~n\", [{expression}])), halt()."
        ),
    ]);
    command
}

/// The `ICANON` and `ECHO` flags of `c_lflag`.
const CANONICAL_ECHO: libc::tcflag_t = libc::ICANON | libc::ECHO;

/// Assert that two runs leave the same mode on each terminal. `termios`
/// has no `PartialEq`, so the test compares its `Debug` text, which shows
/// each field.
///
/// The comparison ignores the `PENDIN` flag of `c_lflag`. It is a status
/// flag of the terminal driver, not a part of the mode that `tty_init/2`
/// saves and sets. The macOS kernel sets it when `ICANON` comes back, and
/// clears it when a reader or a poll of the terminal processes the pending
/// input. After `tty_select/2`, the ERTS poll thread watches fd 0, so the
/// final `PENDIN` of fd 0 in ERTS depends on the thread timing (measured:
/// unset in 2 of 30 ERTS runs under CPU load, set in each PBeam run).
///
/// # Examples
///
/// ```rust,ignore
/// assert_same_modes(&erts_modes, &pbeam_modes);
/// ```
fn assert_same_modes(erts: &[Option<libc::termios>; 3], pbeam: &[Option<libc::termios>; 3]) {
    let without_pendin = |modes: &[Option<libc::termios>; 3]| {
        modes.map(|mode| {
            mode.map(|mut mode| {
                mode.c_lflag &= !libc::PENDIN;
                mode
            })
        })
    };
    assert_eq!(
        format!("{:?}", without_pendin(erts)),
        format!("{:?}", without_pendin(pbeam))
    );
}

/// Assert that `mode` has the raw changes of `tty_init/2`
/// (prim_tty_nif.c:1108-1128).
///
/// # Examples
///
/// ```rust,ignore
/// assert_raw(&stderr);
/// ```
fn assert_raw(mode: &libc::termios) {
    assert_eq!(mode.c_iflag & (libc::ISTRIP | libc::ICRNL), 0);
    assert_eq!(mode.c_lflag & CANONICAL_ECHO, 0);
    assert_eq!(mode.c_oflag & libc::OPOST, 0);
    assert_eq!((mode.c_cc[libc::VMIN], mode.c_cc[libc::VTIME]), (1, 0));
}

/// Run `tty_nif_probe:Function()` on the pinned ERTS with the stand-in
/// `prim_tty` and the standard streams of `layout`, and return the `~w`
/// text of its result. The probe writes with `file:write_file/2`, because
/// the boot IO servers call the stand-in after `code:load_abs/1`. With
/// `-noinput`, no boot IO server reads fd 0, as in PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(run_erts_nif_probe(FILES, &[], "isatty"), b"[false,false,false,badarg,badarg,badarg]\n");
/// ```
fn run_erts_nif_probe(
    layout: [Stream; 3],
    environment: &[(&str, &str)],
    function: &str,
) -> Vec<u8> {
    run_in_layout(&mut erts_nif_probe(function), layout, environment)
}

/// Return the `erl` command of `run_erts_nif_probe`.
///
/// # Examples
///
/// ```rust,ignore
/// let (result, modes) = run_with_modes(&mut erts_nif_probe("init"), OUTPUT_TERMINALS, &[]);
/// ```
fn erts_nif_probe(function: &str) -> Command {
    erts_stand_in_probe(
        "nif",
        &format!("begin ok = prim_tty:load(\"prim_tty\"), tty_nif_probe:{function}() end"),
    )
}

/// Return the `erl` command that runs `tty_init_probe:main/0` with the
/// pinned `prim_tty` and writes the `~w` text of its result to
/// `standard_error`.
///
/// # Examples
///
/// ```rust,ignore
/// let outputs = run_with_outputs(&mut erts_init_probe(), FILES, &[]);
/// ```
fn erts_init_probe() -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noshell",
        "-pa",
        "fixtures/prim_tty",
        "-eval",
        "io:format(standard_error, \"~w~n\", [tty_init_probe:main()]), halt().",
    ]);
    command
}

/// Return the `erl` command that runs `host_write_probe:main/0` with the
/// pinned `prim_tty`.
///
/// # Examples
///
/// ```rust,ignore
/// let outputs = run_with_outputs(&mut erts_host_write_probe(), FILES, &[]);
/// ```
fn erts_host_write_probe() -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noshell",
        "-pa",
        "fixtures/prim_tty",
        "-eval",
        "host_write_probe:main(), halt().",
    ]);
    command
}

/// Return the `erl` command that runs `cooked_probe:Function()` with the
/// pinned `prim_tty` and writes the `~w` text of its result to
/// `standard_error`. It uses `-noinput`: under `-noshell`, `user_drv` starts
/// its own cooked reader on fd 0 (user_sup.erl:129-130), and the select of
/// the probe reader makes ERTS log "stealing control of fd=0" on stdout
/// (erl_check_io.c:1608). Under `-noinput`, no other process reads fd 0
/// (user_sup.erl:135-136), as in PBeam.
///
/// # Examples
///
/// ```rust,ignore
/// let outputs = run_with_outputs(&mut erts_cooked_probe("main"), FILES, &[]);
/// ```
fn erts_cooked_probe(function: &str) -> Command {
    let mut command = Command::new("erl");
    command.args([
        "-noinput",
        "-pa",
        "fixtures/prim_tty",
        "-eval",
        &format!("io:format(standard_error, \"~w~n\", [cooked_probe:{function}()]), halt()."),
    ]);
    command
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
/// let result = run_isolated(FILES, &[], "tty_nif_probe:isatty", &[TTY_NIF_PROBE, STAND_IN]);
/// assert!(result.starts_with(b"Ok(List(["));
/// ```
fn run_isolated(
    layout: [Stream; 3],
    environment: &[(&str, &str)],
    entry: &str,
    beams: &[&str],
) -> Vec<u8> {
    run_in_layout(&mut isolated(entry, beams), layout, environment)
}

/// Return the child command of `run_isolated`.
///
/// # Examples
///
/// ```rust,ignore
/// let command = isolated("tty_nif_probe:init", &[TTY_NIF_PROBE, STAND_IN]);
/// let (result, modes) = run_with_modes(&mut command, OUTPUT_TERMINALS, &[]);
/// ```
fn isolated(entry: &str, beams: &[impl AsRef<OsStr>]) -> Command {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--exact", "isolated_run", "--include-ignored", "--quiet"])
        .env(ENTRY, entry)
        .env(BEAMS, env::join_paths(beams).unwrap());
    command
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

/// Run `command` with `TERM=dumb`, `LC_ALL=C.UTF-8`, the variables of
/// `environment`, which replace these values, and the standard streams of
/// `layout`, and return the bytes that it writes to the file in
/// the `PBEAM_TTY_RESULT` variable. Each pseudo-terminal master stays open
/// until the command exits.
///
/// # Examples
///
/// ```rust,ignore
/// let result = run_in_layout(Command::new("erl").args(["-noshell", "-eval", "..."]), FILES, &[]);
/// ```
fn run_in_layout(
    command: &mut Command,
    layout: [Stream; 3],
    environment: &[(&str, &str)],
) -> Vec<u8> {
    run_with_modes(command, layout, environment).0
}

/// Run `command` as `run_in_layout` does, and also return the mode of each
/// pseudo-terminal of `layout` after the command exits, by fd. A stream
/// that is not a terminal gives `None`.
///
/// # Examples
///
/// ```rust,ignore
/// let (result, [_, stdout, stderr]) = run_with_modes(&mut erts_nif_probe("init"), OUTPUT_TERMINALS, &[]);
/// assert!(stdout.is_some());
/// ```
fn run_with_modes(
    command: &mut Command,
    layout: [Stream; 3],
    environment: &[(&str, &str)],
) -> (Vec<u8>, [Option<libc::termios>; 3]) {
    let outputs = run_with_outputs(command, layout, environment);
    (outputs.result.unwrap(), outputs.modes)
}

/// What one command leaves after it exits in a layout.
struct Outputs {
    /// The bytes of the file in the `PBEAM_TTY_RESULT` variable, or `None`
    /// when the command does not write it.
    result: Option<Vec<u8>>,
    /// The mode of each pseudo-terminal of the layout, by fd.
    modes: [Option<libc::termios>; 3],
    /// The bytes of each file stream of the layout, by fd.
    files: [Option<Vec<u8>>; 3],
}

/// Run `command` with `TERM=dumb`, `LC_ALL=C.UTF-8`, the variables of
/// `environment`, which replace these values, and the standard streams of
/// `layout`, and return what it leaves after it exits. Each pseudo-terminal
/// master stays open until the command exits.
///
/// # Examples
///
/// ```rust,ignore
/// let outputs = run_with_outputs(&mut erts_init_probe(), FILES, &[]);
/// assert_eq!(outputs.files[1].as_deref(), Some(&b""[..]));
/// ```
fn run_with_outputs(
    command: &mut Command,
    layout: [Stream; 3],
    environment: &[(&str, &str)],
) -> Outputs {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let directory = env::temp_dir().join(format!(
        "pbeam-prim-tty-{}-{}",
        std::process::id(),
        RUNS.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&directory).unwrap();
    let paths: [_; 3] = std::array::from_fn(|fd| directory.join(format!("fd{fd}")));
    let mut terminals: [Option<(OwnedFd, OwnedFd)>; 3] = Default::default();
    let [stdin, stdout, stderr] =
        std::array::from_fn(|fd| open_stream(layout[fd], &paths[fd], &mut terminals[fd]));
    let result = directory.join("result");
    let closed = layout.map(|stream| matches!(stream, Stream::Closed));
    if closed.contains(&true) {
        // SAFETY: the closure calls only `close`, which is async-signal-safe,
        // and allocates nothing. `pre_exec` runs after the standard streams
        // are set, so each `Closed` fd stays closed at `exec`.
        unsafe {
            command.pre_exec(move || {
                for (fd, closed) in (0..).zip(closed) {
                    if closed {
                        libc::close(fd);
                    }
                }
                Ok(())
            });
        }
    }
    let status = command
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .envs(environment.iter().copied())
        .env(RESULT, &result)
        .stdin(stdin)
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .unwrap();
    // Keep the files of a failed run for diagnosis.
    assert!(
        status.success(),
        "{command:?} exited with {status}; its files are in {}",
        directory.display()
    );
    let outputs = Outputs {
        result: fs::read(&result).ok(),
        modes: terminals.map(|terminal| terminal.map(|(_, slave)| mode(&slave))),
        files: std::array::from_fn(|fd| {
            matches!(layout[fd], Stream::File).then(|| fs::read(&paths[fd]).unwrap())
        }),
    };
    fs::remove_dir_all(&directory).unwrap();
    outputs
}

/// Open one standard stream for a child. A file stream creates `path`. A
/// terminal stream keeps the master of its pseudo-terminal and a copy of
/// its slave in `terminal`, so the pseudo-terminal stays open and its mode
/// can be read after the child exits.
///
/// # Examples
///
/// ```rust,ignore
/// let mut terminal = None;
/// let stdout = open_stream(Stream::Terminal { columns: 100, rows: 30 }, &path, &mut terminal);
/// assert!(terminal.is_some());
/// ```
fn open_stream(stream: Stream, path: &Path, terminal: &mut Option<(OwnedFd, OwnedFd)>) -> Stdio {
    let (master, slave) = match stream {
        // `run_with_outputs` closes the fd of a `Closed` stream in the child.
        Stream::Null | Stream::Closed => return Stdio::null(),
        Stream::File => return fs::File::create(path).unwrap().into(),
        // `std::io::pipe` makes both ends close-on-exec, so a child of a
        // parallel test does not keep the read end open.
        Stream::BrokenPipe => return std::io::pipe().unwrap().1.into(),
        Stream::Terminal { columns, rows } => open_terminal(columns, rows),
        Stream::QuietTerminal { columns, rows } => {
            let (master, slave) = open_terminal(columns, rows);
            let mut quiet = mode(&slave);
            quiet.c_lflag &= !(libc::ECHO | libc::ICANON);
            // SAFETY: `tcsetattr` reads one `termios` from the pointer.
            assert_eq!(
                unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &quiet) },
                0
            );
            (master, slave)
        }
    };
    let child = slave.try_clone().unwrap();
    *terminal = Some((master, slave));
    child.into()
}

/// Read the mode of the terminal `fd` with `tcgetattr`.
///
/// # Examples
///
/// ```rust,ignore
/// let (_master, slave) = open_terminal(80, 24);
/// assert_ne!(mode(&slave).c_lflag & libc::ECHO, 0);
/// ```
fn mode(fd: &OwnedFd) -> libc::termios {
    // SAFETY: `termios` is a C struct of integers, so all zeros is a valid
    // value, and `tcgetattr` writes one `termios` to the pointer.
    unsafe {
        let mut mode: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(fd.as_raw_fd(), &raw mut mode), 0);
        mode
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
