//! Runs the pinned OTP IO request path down to the writer NIF on PBeam.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const IO_PATH_PROBE: &[u8] = include_bytes!("../../fixtures/io_path/io_path_probe.beam");

#[test]
fn executes_the_pinned_otp_io_request_path_to_the_writer_nif() {
    // `main/0` makes a pinned `group` the group leader of the caller, with a
    // pinned `user_drv` as its driver, and calls `io:put_chars/2`. The writer
    // of `user_drv` runs `prim_tty:writer_loop/2`, which calls the local
    // `write_nif/2` (prim_tty.erl:560). The writer exits and is not linked.
    // `user_drv` gets the `DOWN` message, replies `{error, Reason}` to the
    // group (user_drv.erl:566-573), and stops with `normal`. Its `?LOG_INFO`
    // runs `logger_config:allow(info, user_drv)`, which reads the
    // `persistent_term` `?NOTICE` default and returns `false`. The group
    // forwards the reply (group.erl:506-508) and stops with `normal` when its
    // linked driver exits (group.erl:522-525). `io:o_request/2` raises
    // `error:badarg` (io.erl:203-207, 785).
    // The noise message stays in the mailbox of the caller.
    //
    // `io:conv_reason/1` gives `badarg` for most writer exit reasons, so the
    // caller result alone does not show where the writer failed. `main/0`
    // also monitors the writer and returns its exit reason. The `nif_start`
    // instruction of `write_nif/2` runs the PBeam binding. The binding raises
    // `badarg` for the plain reference, as the ERTS NIF does for a term that
    // is not a TTY resource (prim_tty_nif.c:363-364). On ERTS the reason is
    // `{badarg, [{prim_tty, write_nif, [Ref, Chars], []}, ...]}`, as
    // `dirty_nif_exception` restores the MFA of the NIF
    // (erl_nif.c:3431-3433). PBeam gives the same first entry; its later
    // frames differ (row "Exception stack trace"), so both checks compare the
    // reason and the first entry.
    let modules = [
        "erlang",
        "erts_internal",
        "io",
        "gen_statem",
        "gen",
        "group",
        "user_drv",
        "prim_tty",
        "queue",
        "unicode",
        "logger",
        "logger_config",
    ]
    .map(pinned_beam);
    let mut beams = vec![IO_PATH_PROBE];
    beams.extend(modules.iter().map(Vec::as_slice));
    let program = BeamProgram::load(&beams).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let result = program.execute("io_path_probe", "main", &[]).unwrap().term;
    let Term::Tuple(ref result) = result else {
        panic!("main/0 gives a tuple");
    };
    assert_eq!(
        result[..4],
        [
            Term::tuple(vec![atom("error"), atom("badarg")]),
            atom("noise_ok"),
            atom("normal"),
            atom("normal"),
        ]
    );
    let Term::Tuple(ref writer) = result[4] else {
        panic!("the writer exit reason is a tuple: {:?}", result[4]);
    };
    let first = writer[1].proper_list().unwrap().next().unwrap().clone();
    let Term::Tuple(ref first) = first else {
        panic!("a stack trace entry is a tuple: {first:?}");
    };
    assert_eq!(
        (&writer[0], &first[0], &first[1], &first[3]),
        (
            &atom("badarg"),
            &atom("prim_tty"),
            &atom("write_nif"),
            &Term::nil()
        )
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_path",
            "-eval",
            "{R, K, D, G, W} = io_path_probe:main(), \
             F = case W of {C, [{M, N, _, _} | _]} -> {C, M, N}; _ -> W end, \
             io:format(\"~p~n\", [{R, K, D, G, F}]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, <[u8; 0]>::default());
    assert_eq!(
        without_writer_report(&String::from_utf8(reference.stdout).unwrap()),
        "{{error,badarg},noise_ok,normal,normal,{badarg,prim_tty,write_nif}}\n"
    );
}

/// Remove the one asynchronous ERTS error report of the failed `prim_tty`
/// writer from the reference stdout, if the report is there. The report
/// starts with `=ERROR REPORT==== ` and ends with the empty line after the
/// `prim_tty:writer_loop/2` frame. Its timing changes between runs, and some
/// runs halt before it is printed. A second report stays in the result.
///
/// # Examples
///
/// ```rust,ignore
/// let stdout = "ok\n=ERROR REPORT==== ...\nError in process <0.89.0> \
///     with exit value:\n{badarg,[{prim_tty,write_nif,...},\n \
///     {prim_tty,writer_loop,2,[{file,\"prim_tty.erl\"},{line,560}]}]}\n\n";
/// assert_eq!(without_writer_report(stdout), "badarg\n");
/// assert_eq!(without_writer_report("badarg\n"), "badarg\n");
/// ```
fn without_writer_report(stdout: &str) -> String {
    const END: &str = "{prim_tty,writer_loop,2,[{file,\"prim_tty.erl\"},{line,560}]}]}\n\n";
    let Some(start) = stdout.find("=ERROR REPORT==== ") else {
        return stdout.to_owned();
    };
    let end = start + stdout[start..].find(END).unwrap() + END.len();
    let report = &stdout[start..end];
    assert!(report.contains("with exit value:"), "{report}");
    assert!(report.contains("{badarg,[{prim_tty,write_nif,"), "{report}");
    format!("{}{}", &stdout[..start], &stdout[end..])
}
