//! Runs the BIFs of the `event_bif` probe (integer to list, iolist size, remainder, minimum,
//! printable range) on PBeam and compares them with the pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const EVENT_BIF_PROBE: &[u8] = include_bytes!("../../fixtures/event_bif/event_bif_probe.beam");

/// Run `event_bif_probe:Function()` on the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    evaluate(&format!("event_bif_probe:{function}()"))
}

/// Evaluate `expression` on the pinned ERTS, with the fixture on the code
/// path, and return its `~w` output.
fn evaluate(expression: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/event_bif",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    reference.stdout
}

/// Return `count` copies of the tuple `{error, Reason}` of `program`.
fn errors(program: &BeamProgram, reason: &str, count: usize) -> impl Iterator<Item = Term> {
    let error = Term::tuple(vec![
        Term::Atom(program.atom("error").unwrap()),
        Term::Atom(program.atom(reason).unwrap()),
    ]);
    std::iter::repeat_n(error, count)
}

#[test]
fn converts_integers_to_lists_as_pinned_erts() {
    // `event_bif_probe:to_list/0` runs `erlang:integer_to_list/1`, as
    // `io_lib:write1/4` does. 2^59 - 1 and -2^59 are the limits of an ERTS
    // small integer, and 2^63 - 1 and -2^63 are ERTS bignums, with the same
    // strings. A term that is not an integer raises
    // `badarg` (`integer_to_list_1`, `bif.c:3313-3339`).
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let mut expected: Vec<Term> = [
        "0",
        "7",
        "-15",
        "1000000",
        "576460752303423487",
        "-576460752303423488",
        "9223372036854775807",
        "-9223372036854775808",
    ]
    .map(|digits| Term::list_from(digits.bytes().map(|byte| Term::Integer(byte.into()))))
    .into();
    expected.extend(errors(&program, "badarg", 4));
    assert_eq!(
        program
            .execute("event_bif_probe", "to_list", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("to_list"),
        b"[[48],[55],[45,49,53],[49,48,48,48,48,48,48],\
          [53,55,54,52,54,48,55,53,50,51,48,51,52,50,51,52,56,55],\
          [45,53,55,54,52,54,48,55,53,50,51,48,51,52,50,51,52,56,56],\
          [57,50,50,51,51,55,50,48,51,54,56,53,52,55,55,53,56,48,55],\
          [45,57,50,50,51,51,55,50,48,51,54,56,53,52,55,55,53,56,48,56],\
          {error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn counts_iolist_bytes_as_pinned_erts() {
    // `event_bif_probe:size/0` runs `erlang:iolist_size/1`, as
    // `io_lib:chars_length/1` does, on binaries, proper and improper
    // iolists, and nested lists. An element that is not a byte, a binary, or
    // a list, a tail that is not a binary or `[]`, and a term that is not
    // iodata raise `badarg` (`generic_iolist_size`, `erl_iolist.c:153-236`).
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let mut expected: Vec<Term> = [0, 0, 3, 3, 4, 3, 2].map(Term::Integer).into();
    expected.extend(errors(&program, "badarg", 8));
    assert_eq!(
        program
            .execute("event_bif_probe", "size", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("size"),
        b"[0,0,3,3,4,3,2,{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
          {error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn takes_integer_remainders_as_pinned_erts() {
    // `event_bif_probe:remainder/0` runs `erlang:rem/2`, as
    // `logger_formatter:timestamp_to_datetimemicro/2` does. The remainder has
    // the sign of the dividend (`rem_2`, `erl_arith.c:145-161`), also for
    // ERTS bignum operands, and `-2^63 rem -1` gives `0`. A divisor of `0`
    // raises `badarith` before the type check (`erl_arith.c:149-151`), so
    // `a rem 0` raises `badarith`, and an operand that is not an integer
    // raises `badarith`.
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let mut expected: Vec<Term> = [1, -1, 1, -1, 0, 5, 7, 0, -1].map(Term::Integer).into();
    expected.extend(errors(&program, "badarith", 5));
    assert_eq!(
        program
            .execute("event_bif_probe", "remainder", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("remainder"),
        b"[1,-1,1,-1,0,5,7,0,-1,{error,badarith},{error,badarith},{error,badarith},\
          {error,badarith},{error,badarith}]\n"
    );
}

#[test]
fn selects_the_minimum_as_pinned_erts() {
    // `event_bif_probe:minimum/0` runs `erlang:min/2`, as
    // `io_lib_format:term_bin/8` does. The result is the second argument only
    // when the first is greater in the term order, so `min(1, 1)` and
    // `min(Ref, Ref)` give the first argument (`min_2`,
    // `erl_bif_guard.c:475-479`).
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("event_bif_probe", "minimum", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::Integer(1),
            Term::Integer(1),
            Term::Integer(1),
            Term::Integer(1),
            Term::Integer(1),
            atom("a"),
            Term::tuple(vec![Term::Integer(1)]),
            Term::tuple(vec![Term::Integer(1)]),
            Term::binary(b"a".to_vec()),
            atom("true"),
            atom("a"),
            Term::tuple(vec![]),
        ])
    );
    assert_eq!(
        reference("minimum"),
        b"[1,1,1,1,1,a,{1},{1},<<97>>,true,a,{}]\n"
    );
}

#[test]
fn gives_the_minimum_of_two_references() {
    // `event_bif_probe:refs/0` runs `erlang:min/2` on two references that
    // one process makes. ERTS gives the reference with the smaller number
    // words (`erts_cmp_compound`, utils.c:2239-2290), so PBeam gives a
    // reference too, as the "Term order" row of `docs/erts-differences.md`
    // records.
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let term = program
        .execute("event_bif_probe", "refs", &[])
        .unwrap()
        .term;
    assert!(matches!(term, Term::Reference(_)), "{term:?}");
    assert_eq!(evaluate("is_reference(event_bif_probe:refs())"), b"true\n");
}

#[test]
fn gives_the_default_printable_range_as_pinned_erts() {
    // `event_bif_probe:range/0` runs `io:printable_range/0`, as
    // `io_lib:printable_list/1` does. ERTS gives `latin1` without the
    // emulator flag `+pc unicode` (`io_printable_range_0`,
    // `erl_unicode.c:2943-2950`), and PBeam takes no emulator flags.
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("event_bif_probe", "range", &[])
            .unwrap()
            .term,
        Term::Atom(program.atom("latin1").unwrap())
    );
    assert_eq!(reference("range"), b"latin1\n");
}

#[test]
fn checks_process_liveness_as_pinned_erts() {
    // `event_bif_probe:alive/0` runs `erlang:is_process_alive/1`, as
    // `logger_h_common:log/2` does. The caller and a waiting process give
    // `true`, a process after its `DOWN` message gives `false`, and a term
    // that is not a pid raises `badarg` (`is_process_alive_1`,
    // `erl_bif_info.c:4060-4114`).
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let mut expected = vec![atom("true"), atom("false"), atom("true")];
    expected.extend(errors(&program, "badarg", 5));
    assert_eq!(
        program
            .execute("event_bif_probe", "alive", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("alive"),
        b"[true,false,true,{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
          {error,badarg}]\n"
    );
}

#[test]
fn counts_queued_messages_as_pinned_erts() {
    // `event_bif_probe:queue_len/0` runs `erlang:process_info/2` with the
    // item `message_queue_len`, as `logger_olp:flush/2` and
    // `logger_olp:check_load/1` do, for the caller and for a process that
    // waits for another message. The count holds each message that is still
    // in the queue (`erl_bif_info.c:1742-1754`). The other process has
    // messages that it did not fetch, so it gets a process-info request,
    // and the caller waits for the reply in `erts_internal:await_result/1`
    // (`erl_bif_info.c:1478-1500`).
    let erts_internal = pinned_beam("erts_internal");
    let program = BeamProgram::load(&[&erts_internal, EVENT_BIF_PROBE]).unwrap();
    let item = Term::Atom(program.atom("message_queue_len").unwrap());
    let expected = [0, 2, 1, 2].map(|count| Term::tuple(vec![item.clone(), Term::Integer(count)]));
    assert_eq!(
        program
            .execute("event_bif_probe", "queue_len", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(
        reference("queue_len"),
        b"[{message_queue_len,0},{message_queue_len,2},{message_queue_len,1},\
          {message_queue_len,2}]\n"
    );
}

#[test]
fn gets_persistent_terms_as_pinned_erts() {
    // `event_bif_probe:term/0` runs `persistent_term:get/1`, as
    // `erl_features:keywords/0` does. A stored key gives its value, and a
    // missing key raises `badarg` (`persistent_term_get_1`,
    // `erl_bif_persistent.c:380-388`).
    let program = BeamProgram::load(&[EVENT_BIF_PROBE]).unwrap();
    let mut expected = vec![Term::Atom(program.atom("value").unwrap())];
    expected.extend(errors(&program, "badarg", 1));
    assert_eq!(
        program
            .execute("event_bif_probe", "term", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
    assert_eq!(reference("term"), b"[value,{error,badarg}]\n");
}
