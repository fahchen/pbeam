//! Runs `list_to_integer` on PBeam and compares the results with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const LIST_TO_INTEGER_PROBE: &[u8] =
    include_bytes!("../../fixtures/list_to_integer/list_to_integer_probe.beam");

/// `list_to_integer_probe:main()` on the pinned ERTS, as `io:format("~w")`
/// prints it.
const ERTS_RESULT: &str = "{[no_integer,not_a_list,not_a_list,{12,[]},{-12,[97]},{7,[]},\
    no_integer,no_integer,{0,[]},{0,[97]},{0,[97]},{10,[]},{1295,[]},{1,[50]},badarg,badarg,\
    badarg,badarg,{1,[]},{1,[]},no_integer,{12345678901234567,[]},big,{-1,[]},{1,foo},\
    no_integer,no_integer,{1,[[50]]},no_integer,{0,b},{0,b},{10,b},no_integer],\
    [42,{error,badarg},{error,badarg},-127,{error,badarg}]}";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe result.
///
/// The atom names come from `atoms`, the table that owns the term.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::Integer(1)]), &atoms), "[1]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn reads_integers_from_lists_as_pinned_erts() {
    // `erts_internal:list_to_integer/2` reads a sign and the digits of the
    // base, and returns `{Integer, Rest}`, `no_integer`, `not_a_list`,
    // `badarg`, or `big` (big.c:4258-4354). It reads the low 8 bits of each
    // small integer element. `erlang:list_to_integer/1,2` of `erlang.beam`
    // call it (erlang.erl:4111-4131, 4165-4182). `code_server` reads the
    // version numbers of the OTP `lib` directories with it
    // (code_server.erl:409-415).
    let program = BeamProgram::load(&[&pinned_beam("erlang"), LIST_TO_INTEGER_PROBE]).unwrap();
    let result = program
        .execute("list_to_integer_probe", "main", &[])
        .unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_RESULT);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/list_to_integer",
            "-eval",
            "io:format(\"~w~n\", [list_to_integer_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{ERTS_RESULT}\n")
    );
}
