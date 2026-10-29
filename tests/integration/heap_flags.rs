//! The heap flags of `process_flag/2` and `system_flag/2`: PBeam accepts
//! them with the value checks of ERTS, stores them on the process or the
//! node, and reports them. PBeam has no garbage collector, so the values
//! change only what the BIFs report (row "Heap limit" in
//! `docs/erts-differences.md`).

#[path = "../common/mod.rs"]
mod common;

use std::fmt::Write as _;
use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AtomTable, BeamProgram, Term};

const HEAP_FLAGS_PROBE: &[u8] = include_bytes!("../../fixtures/heap_flags/heap_flags_probe.beam");

/// The default max heap size map as the probe lists it.
const DEFAULT_MAX: &str =
    "[{error_logger,true},{include_shared_binaries,false},{kill,true},{size,0}]";

/// Return the results of each probe function, as `~w` prints them on the
/// pinned ERTS.
fn results() -> Vec<(&'static str, String)> {
    let small = "[{error_logger,true},{include_shared_binaries,false},{kill,false},{size,500}]";
    vec![
        (
            "process_flags",
            format!(
                "{{[233,233,376,46422,46422,75113,65535,34464,{DEFAULT_MAX},\
                 [{{error_logger,true}},{{include_shared_binaries,false}},{{kill,false}},{{size,1000}}],\
                 [{{error_logger,false}},{{include_shared_binaries,true}},{{kill,true}},{{size,2000}}],233],\
                 [6772,46422,3,{DEFAULT_MAX}],\
                 [{{max_heap_size,{DEFAULT_MAX}}},{{min_bin_vheap_size,46422}},{{min_heap_size,6772}},\
                 {{fullsweep_after,3}}]}}"
            ),
        ),
        (
            "process_errors",
            format!("[badarg,badarg,badarg,badarg,badarg,233,badarg,{DEFAULT_MAX}]"),
        ),
        (
            "heap_items",
            format!(
                "{{[true,true,true],[true,true,true],{{garbage_collection,[{{max_heap_size,{DEFAULT_MAX}}},\
                 {{min_bin_vheap_size,46422}},{{min_heap_size,376}},{{fullsweep_after,3}},{{minor_gcs,0}}]}}}}"
            ),
        ),
        ("queue_costs", "[100,100,0,0,0,true]".to_owned()),
        (
            "system_flags",
            format!(
                "{{[233,12,46422,65535,65535,{DEFAULT_MAX}],{{[376,12,7,{small}],376,{small}}},badarg,12,\
                 [376,507125831,7,{small}],[376,507125831,7,{small}]}}"
            ),
        ),
    ]
}

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe.
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: Vec<&Term>| {
        terms
            .into_iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        _ if let Some(terms) = term.proper_list() => format!("[{}]", join(terms.collect())),
        Term::Tuple(terms) => format!("{{{}}}", join(terms.iter().collect())),
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn sets_and_reports_the_heap_flags_as_pinned_erts() {
    let erlang = pinned_beam("erlang");
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erlang, &erts_internal, HEAP_FLAGS_PROBE]).unwrap();
    let results = results();
    for (function, expected) in &results {
        let result = program.execute("heap_flags_probe", function, &[]).unwrap();
        assert_eq!(text(&result.term, result.atoms()), *expected, "{function}");
    }

    let calls: Vec<String> = results
        .iter()
        .map(|(function, _)| format!("io:format(\"~w~n\", [heap_flags_probe:{function}()])"))
        .collect();
    let reference = Command::new("erl")
        .args(["-noshell", "-pa", "fixtures/heap_flags", "-eval"])
        .arg(format!("{}, halt().", calls.join(", ")))
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    let expected: String = results.iter().fold(String::new(), |mut text, (_, result)| {
        let _ = writeln!(text, "{result}");
        text
    });
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), expected);
}

#[test]
fn gives_a_runtime_error_for_a_heap_size_above_the_largest_heap_size() {
    // `erts_next_heap_size` stops the emulator for a size at or above the
    // largest heap size (`erl_gc.c:335`), so no ERTS reference exists.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, HEAP_FLAGS_PROBE]).unwrap();
    for case in ["process", "system"] {
        let argument = program.host_term(Term::Atom(program.atom(case).unwrap()));
        let error = program
            .execute("heap_flags_probe", "huge", &[argument])
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("no next heap size found: 36028797018963968"),
            "{error}"
        );
    }
}
