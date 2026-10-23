//! The options of `erlang:spawn_opt/4`: PBeam accepts each option of ERTS
//! `erts_parse_spawn_opts` (`erl_process.c:12222-12434`), sets the start
//! flags of the new process, and raises `badarg` for each option that is not
//! valid (`bif.c:1039-1051`).

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{AtomTable, BeamProgram, Term};

const SPAWN_OPT_PROBE: &[u8] = include_bytes!("../../fixtures/spawn_opt/spawn_opt_probe.beam");

/// Return the start flags of one probe case: the heap flags, the priority,
/// the max heap size, `async_dist`, and `message_queue_data`.
fn flags(heap: &str, priority: &str, max: &str, async_dist: &str, queue: &str) -> String {
    let (kill, size) = max.split_once(',').unwrap();
    format!(
        "[{heap},{priority},[{{error_logger,true}},{{include_shared_binaries,false}},\
         {{kill,{kill}}},{{size,{size}}}],{async_dist},{queue}]"
    )
}

/// Return the results of each probe function, as `~w` prints them on the
/// pinned ERTS.
fn results() -> Vec<(&'static str, String)> {
    let default = "233,46422,65535";
    let cases = [
        flags(default, "normal", "true,0", "false", "on_heap"),
        flags("1598,46422,65535", "normal", "true,0", "false", "on_heap"),
        flags(default, "normal", "true,0", "false", "on_heap"),
        flags(default, "normal", "true,0", "false", "on_heap"),
        flags("233,121536,5", "normal", "true,0", "false", "on_heap"),
        flags(default, "high", "true,0", "false", "on_heap"),
        flags("233,121536,65535", "normal", "true,0", "false", "on_heap"),
        flags("233,46422,4464", "normal", "true,0", "false", "on_heap"),
        flags("233,46422,9", "normal", "true,0", "false", "on_heap"),
        flags(default, "normal", "false,5000", "false", "on_heap"),
        flags("233,318187,65535", "normal", "true,0", "false", "on_heap"),
        flags(default, "normal", "true,0", "true", "on_heap"),
        flags(default, "normal", "true,0", "false", "on_heap"),
        flags(default, "normal", "true,0", "false", "on_heap"),
        flags(default, "low", "true,0", "false", "off_heap"),
    ];
    vec![
        ("flags", format!("[{}]", cases.join(","))),
        (
            "links",
            "[{true,none},{true,none},{false,down},{false,down},{true,t}]".to_owned(),
        ),
        ("errors", format!("[{}]", vec!["badarg"; 29].join(","))),
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
fn accepts_each_spawn_option_as_pinned_erts() {
    let erlang = pinned_beam("erlang");
    let (lists, maps) = (pinned_beam("lists"), pinned_beam("maps"));
    let program = BeamProgram::load(&[&erlang, &lists, &maps, SPAWN_OPT_PROBE]).unwrap();
    let results = results();
    for (function, expected) in &results {
        let result = program.execute("spawn_opt_probe", function, &[]).unwrap();
        assert_eq!(text(&result.term, result.atoms()), *expected, "{function}");
    }

    let calls: Vec<String> = results
        .iter()
        .map(|(function, _)| format!("io:format(\"~w~n\", [spawn_opt_probe:{function}()])"))
        .collect();
    let reference = Command::new("erl")
        .args(["-noshell", "-pa", "fixtures/spawn_opt", "-eval"])
        .arg(format!("{}, halt().", calls.join(", ")))
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    let expected: String = results
        .iter()
        .map(|(_, result)| format!("{result}\n"))
        .collect();
    assert_eq!(String::from_utf8(reference.stdout).unwrap(), expected);
}

#[test]
fn gives_a_runtime_error_for_an_option_that_pbeam_does_not_run() {
    // ERTS accepts the monitor and link options; PBeam has no alias or
    // priority monitor or link at spawn (row "Spawn options" in
    // `docs/erts-differences.md`). For a heap size at or above the largest
    // heap size, `erts_next_heap_size` stops the emulator (`erl_gc.c:335`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, SPAWN_OPT_PROBE]).unwrap();
    for (case, message) in [
        (
            "monitor",
            r#"unsupported spawn option Tuple([Atom("monitor"), List([Tuple([Atom("alias")"#,
        ),
        (
            "link",
            r#"unsupported spawn option Tuple([Atom("link"), List([Atom("priority")])])"#,
        ),
        (
            "min_heap_size",
            "no next heap size found: 36028797018963968",
        ),
        (
            "min_bin_vheap_size",
            "no next heap size found: 36028797018963968",
        ),
    ] {
        let argument = program.host_term(Term::Atom(program.atom(case).unwrap()));
        let error = program
            .execute("spawn_opt_probe", "unsupported", &[argument])
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}
