//! Runs the `rand` seeding and drawing functions and the system time on PBeam and compares them
//! with the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

#[path = "../common/mod.rs"]
mod common;

use std::{process::Command, sync::Arc};

use common::pinned_beam;
use num_bigint::BigInt;
use pbeam_runtime::{BeamProgram, Term};

const RAND_PROBE: &[u8] = include_bytes!("../../fixtures/rand/rand_probe.beam");

/// Load the probe with the pinned `rand` module and the modules that its
/// seed and draw paths call.
fn program() -> BeamProgram {
    let modules = ["erlang", "error_handler", "rand", "lists"].map(pinned_beam);
    let mut beams: Vec<&[u8]> = modules.iter().map(Vec::as_slice).collect();
    beams.push(RAND_PROBE);
    BeamProgram::load(&beams).unwrap()
}

/// Run `rand_probe:Function()` on the pinned ERTS and return its `~w`
/// text without the newline.
fn reference(function: &str) -> String {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/rand",
            "-eval",
            &format!("io:format(\"~w~n\", [rand_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// Read one term of `~w` text: an integer, a float, an atom, a list, an
/// improper list cell, or a tuple. An integer outside the i64 range is a
/// `Term::BigInt`, the canonical form. A float parses to the same double
/// only from the same shortest text, so float equality is text equality.
fn parse(text: &mut &str, program: &BeamProgram) -> Term {
    let elements = |text: &mut &str, close: &str| {
        let mut elements = Vec::new();
        while !text.starts_with(close) {
            elements.push(parse(text, program));
            *text = text.strip_prefix(',').unwrap_or(text);
        }
        *text = &text[close.len()..];
        elements
    };
    if let Some(rest) = text.strip_prefix("[]") {
        *text = rest;
        return Term::list_from(Vec::new());
    }
    if let Some(rest) = text.strip_prefix('[') {
        *text = rest;
        let head = parse(text, program);
        if let Some(rest) = text.strip_prefix('|') {
            *text = rest;
            let tail = parse(text, program);
            *text = &text[1..];
            return Term::cons(head, tail);
        }
        *text = text.strip_prefix(',').unwrap_or(text);
        let mut list = vec![head];
        list.extend(elements(text, "]"));
        return Term::list_from(list);
    }
    if let Some(rest) = text.strip_prefix('{') {
        *text = rest;
        return Term::tuple(elements(text, "}"));
    }
    let end = text.find([',', ']', '}', '|']).unwrap_or(text.len());
    let (token, rest) = text.split_at(end);
    *text = rest;
    if token.starts_with(|c: char| c.is_ascii_lowercase()) {
        Term::Atom(program.atom(token).unwrap())
    } else if token.contains('.') {
        Term::Float(token.parse().unwrap())
    } else if let Ok(value) = token.parse::<i64>() {
        Term::Integer(value)
    } else {
        Term::BigInt(Arc::new(token.parse::<BigInt>().unwrap()))
    }
}

/// Assert that `rand_probe:Function()` gives the same term on PBeam as on
/// the pinned ERTS, and return the ERTS text.
fn assert_matches_erts(function: &str) -> String {
    let program = program();
    let erts = reference(function);
    let expected = parse(&mut erts.as_str(), &program);
    let actual = program.execute("rand_probe", function, &[]).unwrap().term;
    assert_eq!(actual, expected, "rand_probe:{function}() on ERTS: {erts}");
    erts
}

#[test]
fn seeds_exsss_and_draws_the_pinned_erts_sequence() {
    let erts = assert_matches_erts("exsss");
    assert!(
        erts.starts_with(
            "{[117085240290607817|199386643319833935],exsss,true,\
             [27,641,475,639,331,474,519,324,913,939,"
        ),
        "{erts}"
    );
    assert!(
        erts.ends_with(",0.761581194244497,0.6015710377718612]}"),
        "{erts}"
    );
}

#[test]
fn seeds_the_default_algorithm_with_an_integer_as_pinned_erts() {
    let erts = assert_matches_erts("default");
    assert!(
        erts.starts_with("{[161132163074061945|185172155811622446],exsss,[1118104649701993671,"),
        "{erts}"
    );
}

#[test]
fn runs_the_time_seeded_draw_of_global_init() {
    let program = program();
    let actual = program.execute("rand_probe", "global", &[]).unwrap().term;
    assert_eq!(actual, parse(&mut "{true,0,true,true,exsss}", &program));
}

#[test]
fn raises_function_clause_for_an_invalid_seed_or_range_as_pinned_erts() {
    assert_eq!(
        assert_matches_erts("invalid"),
        "[{error,function_clause,{rand,mk_alg,foo,1}},\
         {error,function_clause,{rand,exsss_seed,bad,1}},\
         {error,function_clause,{rand,exsss_seed,{1,2},1}},\
         {error,function_clause,{rand,uniform_s,0,2}},\
         {error,function_clause,{rand,uniform_s,-1,2}},\
         {error,function_clause,{rand,uniform_s,1.5,2}}]"
    );
}

#[test]
fn reads_the_erlang_system_time_near_the_os_system_time_as_pinned_erts() {
    // `system_time_1` adds the time offset to the monotonic time
    // (`erl_time_sup.c:2527-2535`), and the offset is the OS system time
    // at the start (`erl_time_sup.c:1244-1249`).
    // A positive bignum unit traps to `erlang:convert_time_unit/3`, and a
    // negative bignum unit raises `badarg` (`erl_time_sup.c:2333-2345`).
    assert_eq!(
        assert_matches_erts("clock"),
        "{true,true,{error,badarg},true,true,1770887431076116955136,{error,badarg}}"
    );
}
