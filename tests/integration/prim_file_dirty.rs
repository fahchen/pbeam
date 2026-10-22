//! The `prim_file` NIFs on the dirty IO pool, with
//! `fixtures/prim_file_dirty/prim_file_dirty_probe.erl`. ERTS runs each
//! `prim_file` NIF except `get_handle_nif/1` on a dirty IO scheduler
//! (prim_file_nif.c:179-218). Each test runs the same function on the pinned
//! ERTS and on PBeam, and compares the `~w` text of the results.

#[path = "../common/mod.rs"]
mod common;

use std::{path::Path, process::Command, sync::mpsc, thread, time::Duration};

use common::pinned_beam;
use pbeam_runtime::{AtomTable, BeamProgram, Term};

const PRIM_FILE_DIRTY_PROBE: &[u8] =
    include_bytes!("../../fixtures/prim_file_dirty/prim_file_dirty_probe.beam");

/// The time that a run has to give its result.
const RUN_TIME_LIMIT: Duration = Duration::from_secs(20);

/// Return the `~w` text of a term of the probe: atoms, integers, binaries
/// as their bytes, lists, and tuples. The atom names come from `atoms`.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::binary(vec![97]), &atoms), "<<97>>");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &mut dyn Iterator<Item = &Term>| {
        terms
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    if let Some(mut terms) = term.proper_list() {
        return format!("[{}]", join(&mut terms));
    }
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::Tuple(terms) => format!("{{{}}}", join(&mut terms.iter())),
        Term::Binary(bytes) => format!(
            "<<{}>>",
            bytes
                .bytes()
                .unwrap()
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        other => panic!("the probe gives no such term: {other:?}"),
    }
}

/// Run `prim_file_dirty_probe:Function(Argument)` on PBeam on a new
/// thread, and return the text of its result. A run that does not end in
/// [`RUN_TIME_LIMIT`] fails the test.
fn pbeam(function: &'static str, argument: &str) -> String {
    let argument = argument.to_owned();
    let (sender, receiver) = mpsc::channel();
    // The program is not `Send`, so the run loads it on its thread.
    thread::spawn(move || {
        let erlang = pinned_beam("erlang");
        let prim_file = pinned_beam("prim_file");
        let lists = pinned_beam("lists");
        let prim_buffer = pinned_beam("prim_buffer");
        let modules = [&erlang, &prim_file, &lists, &prim_buffer];
        let mut beams: Vec<&[u8]> = modules.iter().map(|beam| beam.as_slice()).collect();
        beams.push(PRIM_FILE_DIRTY_PROBE);
        let program = BeamProgram::load(&beams).unwrap();
        let argument = program.host_term(Term::binary(argument.into_bytes()));
        let result = program
            .execute("prim_file_dirty_probe", function, &[argument])
            .unwrap();
        let _ = sender.send(text(&result.term, result.atoms()));
    });
    receiver
        .recv_timeout(RUN_TIME_LIMIT)
        .unwrap_or_else(|error| panic!("{function}: {error}"))
}

/// Run `prim_file_dirty_probe:Function(Argument)` on the pinned ERTS, and
/// return the `~w` text of its result.
fn erts(function: &str, argument: &str) -> String {
    let eval = format!(
        "io:format(\"~w\", [prim_file_dirty_probe:{function}(<<\"{argument}\">>)]), halt()."
    );
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/prim_file_dirty",
            "-eval",
            &eval,
        ])
        .output()
        .unwrap();
    assert_eq!(output.stderr, b"");
    String::from_utf8(output.stdout).unwrap()
}

/// Return a new empty directory for the test `name`.
fn scratch(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "pbeam-prim-file-dirty-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir(&directory).unwrap();
    directory
}

/// Make the FIFO `fifo`.
fn mkfifo(fifo: &Path) {
    assert!(Command::new("mkfifo").arg(fifo).status().unwrap().success());
}

#[test]
fn runs_other_processes_while_a_read_file_waits_as_pinned_erts() {
    // The reader waits in `read_file_nif/1` until the caller opens the
    // FIFO for write. On one scheduler thread, the wait stops the caller,
    // and the run does not end.
    let directory = scratch("fifo");
    let expected = "{running,{ok,<<100,105,114,116,121>>}}";
    for name in ["erts", "pbeam"] {
        let fifo = directory.join(name);
        mkfifo(&fifo);
        let fifo = fifo.to_str().unwrap();
        let result = if name == "erts" {
            erts("fifo", fifo)
        } else {
            pbeam("fifo", fifo)
        };
        assert_eq!(result, expected, "{name}");
    }
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn gives_the_results_of_the_dirty_io_nifs_as_pinned_erts() {
    let directory = scratch("errors");
    let directory = directory.to_str().unwrap();
    let expected = "[{error,enoent},{error,enoent},{error,enoent},{error,enoent},\
                    {error,eisdir},{error,badarg},{ok,0},ok,{error,einval},{error,einval},true]";
    assert_eq!(erts("errors", directory), expected);
    assert_eq!(pbeam("errors", directory), expected);
    std::fs::remove_dir_all(directory).unwrap();
}
