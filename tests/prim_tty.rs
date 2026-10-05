use std::{
    env,
    ffi::OsStr,
    fs,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};

use pbeam_runtime::{BeamProgram, Term};

mod common;

use common::{pinned_beam, pinned_beam_path};

const PRIM_TTY_PROBE: &[u8] = include_bytes!("../fixtures/prim_tty/prim_tty_probe.beam");

/// The stand-in `prim_tty` with the NIF stubs of the pinned module. It is in
/// a subdirectory, so `-pa fixtures/prim_tty` does not load it at boot.
const STAND_IN: &str = "fixtures/prim_tty/nif/prim_tty.beam";
const TTY_NIF_PROBE: &str = "fixtures/prim_tty/tty_nif_probe.beam";
const TTY_INIT_PROBE: &str = "fixtures/prim_tty/tty_init_probe.beam";

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
    // a map, a term that is not a TTY resource, and a map without `input`
    // raise `badarg` in this order (prim_tty_nif.c:1081-1089). On the
    // `stdout` resource, `raw` sets the raw mode, `cooked` sets the first
    // mode again, and `foo` finds the first mode set and does not call
    // `tcsetattr` (prim_tty_nif.c:1099-1134). Last, `raw` on the `stderr`
    // resource. A resource on a file is `unavailable`: `raw` gives
    // `{error, enotsup}` and each other input gives `ok`
    // (prim_tty_nif.c:1091-1095).
    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("init"), FILES, &[]);
    assert_eq!(
        erts,
        b"[badarg,badarg,badarg,badarg,{error,enotsup},ok,ok,{error,enotsup}]\n"
    );
    let probe = || isolated("tty_nif_probe:init", &[TTY_NIF_PROBE, STAND_IN]);
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), FILES, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(List([Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Tuple([Atom("error"), Atom("enotsup")]), Atom("ok"), Atom("ok"), Tuple([Atom("error"), Atom("enotsup")])]))"#
    );
    assert_same_modes(&erts_modes, &pbeam_modes);

    let (erts, erts_modes) = run_with_modes(&mut erts_nif_probe("init"), OUTPUT_TERMINALS, &[]);
    assert_eq!(erts, b"[badarg,badarg,badarg,badarg,ok,ok,ok,ok]\n");
    let (pbeam, pbeam_modes) = run_with_modes(&mut probe(), OUTPUT_TERMINALS, &[]);
    assert_eq!(
        pbeam,
        br#"Ok(List([Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("badarg"), Atom("ok"), Atom("ok"), Atom("ok"), Atom("ok")]))"#
    );
    // The stdout terminal has its first mode again, and the stderr terminal
    // has the raw mode. On macOS the kernel adds `PENDIN` to the stdout
    // mode when `ICANON` comes back, so the modes are compared with the
    // ERTS run.
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
    // Stdin started without `ECHO` and `ICANON`.
    assert_eq!(stdin.c_lflag & CANONICAL_ECHO, CANONICAL_ECHO);
    assert_raw(&stderr);
}

#[test]
fn returns_a_runtime_error_from_write_nif_for_a_tty_resource() {
    // `tty_nif_probe:write/0` makes a `stdout` TTY resource and calls
    // `prim_tty:write_nif/2` with it. ERTS writes the iovec to the output
    // fd (prim_tty_nif.c:366-427). PBeam has no host write yet, so the
    // binding returns a runtime error. `tty_create/1` only reads the mode
    // of stdout, so the test runs in this process.
    let beams = [TTY_NIF_PROBE, STAND_IN].map(|path| fs::read(path).unwrap());
    let program = BeamProgram::load(&beams.each_ref().map(Vec::as_slice)).unwrap();
    let error = program.execute("tty_nif_probe", "write", &[]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "prim_tty:write_nif/2 host write is not implemented yet (#116)"
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
    ]
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
    // output fd (prim_tty_nif.c:1179-1189).
    let outputs = run_with_outputs(&mut erts_init_probe(), STDIN_STDOUT_TERMINALS, &[]);
    assert_eq!(
        outputs.files[2].as_deref(),
        Some(&b"[true,true,false,[true,raw,undefined,true,{ok,{120,40}}],[true,cooked,undefined,true,{ok,{120,40}}],badarg]\n"[..])
    );
    assert_eq!(
        String::from_utf8(run_in_layout(&mut probe(), STDIN_STDOUT_TERMINALS, &[])).unwrap(),
        format!(
            r#"Ok(List([Atom("true"), Atom("true"), Atom("false"), {}]))"#,
            init(
                "true",
                r#"Tuple([Atom("ok"), Tuple([Integer(120), Integer(40)])])"#
            )
        )
    );
}

/// The `ICANON` and `ECHO` flags of `c_lflag`.
const CANONICAL_ECHO: libc::tcflag_t = libc::ICANON | libc::ECHO;

/// Assert that two runs leave the same mode on each terminal. `termios`
/// has no `PartialEq`, so the test compares its `Debug` text, which shows
/// each field.
///
/// # Examples
///
/// ```rust,ignore
/// assert_same_modes(&erts_modes, &pbeam_modes);
/// ```
fn assert_same_modes(erts: &[Option<libc::termios>; 3], pbeam: &[Option<libc::termios>; 3]) {
    assert_eq!(format!("{erts:?}"), format!("{pbeam:?}"));
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
/// `-noinput`, no boot IO server reads fd 0, as in PBeam. A read of a
/// macOS terminal clears the `PENDIN` flag (measured) that the kernel sets when
/// `ICANON` comes back, so a reader changes the final mode of fd 0.
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
    let mut command = Command::new("erl");
    command.args([
        "-noinput",
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
    command
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
    let outputs = Outputs {
        result: fs::read(&result).ok(),
        modes: terminals.map(|terminal| terminal.map(|(_, slave)| mode(&slave))),
        files: std::array::from_fn(|fd| {
            matches!(layout[fd], Stream::File).then(|| fs::read(&paths[fd]).unwrap())
        }),
    };
    fs::remove_dir_all(&directory).unwrap();
    assert!(status.success(), "{command:?} exited with {status}");
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
        Stream::Null => return Stdio::null(),
        Stream::File => return fs::File::create(path).unwrap().into(),
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
