use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const EVENT_BIF_PROBE: &[u8] = include_bytes!("../fixtures/event_bif/event_bif_probe.beam");

/// Run `event_bif_probe:Function()` on the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/event_bif",
            "-eval",
            &format!("io:format(\"~w~n\", [event_bif_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

/// Return `count` copies of the tuple `{error, Reason}` of `program`.
fn errors(program: &BeamProgram, reason: &str, count: usize) -> impl Iterator<Item = Term> {
    let error = Term::Tuple(vec![
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
    .map(|digits| {
        Term::List(
            digits
                .bytes()
                .map(|byte| Term::Integer(byte.into()))
                .collect(),
        )
    })
    .into();
    expected.extend(errors(&program, "badarg", 4));
    assert_eq!(
        program.execute("event_bif_probe", "to_list", &[]).unwrap(),
        Term::List(expected)
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
        program.execute("event_bif_probe", "size", &[]).unwrap(),
        Term::List(expected)
    );
    assert_eq!(
        reference("size"),
        b"[0,0,3,3,4,3,2,{error,badarg},{error,badarg},{error,badarg},{error,badarg},\
          {error,badarg},{error,badarg},{error,badarg},{error,badarg}]\n"
    );
}
