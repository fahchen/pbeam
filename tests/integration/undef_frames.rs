//! The full stack trace of `error:undef` for each call kind, compared with
//! the pinned ERTS. The default error handler raises `undef` through
//! `error_handler:crash/1`, which keeps the tail of the stack trace of
//! `erlang:error(undef)` after the frame of the failed call
//! (`error_handler.erl:165-178`).

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AtomTable, BeamProgram, Term};

const UNDEF_INIT: &[u8] = include_bytes!("../../fixtures/undef/undef_init.beam");
const PROBE: &[u8] = include_bytes!("../../fixtures/undef_frames/undef_frames.beam");
const TARGET: &[u8] = include_bytes!("../../fixtures/undef_frames/undef_frames_target.beam");

#[test]
fn gives_the_full_undef_stack_trace_of_each_call_kind_as_pinned_erts() {
    // `undef_frames:all/0` runs each call kind in a new process after 0 to 9 and 3939 to 3999
    // reductions of `burn/1`, so the frames do not depend on the caller of
    // the probe.
    let program = BeamProgram::load(&[
        &pinned_beam("erlang"),
        &pinned_beam("error_handler"),
        &pinned_beam("init"),
        &pinned_beam("lists"),
        UNDEF_INIT,
        PROBE,
        TARGET,
    ])
    .unwrap();
    let result = program.execute("undef_frames", "all", &[]).unwrap();
    let pbeam = text(&result.term, result.atoms());
    let erts = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-kernel",
            "logger_level",
            "none",
            "-pa",
            "fixtures/undef_frames",
            "-eval",
            "io:format(\"~w~n\", [undef_frames:all()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(erts.status.success(), "{erts:?}");
    assert_eq!(
        format!("{pbeam}\n"),
        String::from_utf8(erts.stdout).unwrap()
    );
}

/// Return the `~w` text of a term of atoms, integers, tuples, and lists.
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => {
            let name = atoms.name_of(atom);
            let plain = name.starts_with(|c: char| c.is_ascii_lowercase())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if plain {
                name.to_owned()
            } else {
                format!("'{name}'")
            }
        }
        Term::Integer(value) => value.to_string(),
        _ if let Some(elements) = term.proper_list() => {
            format!("[{}]", join(&elements.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}
