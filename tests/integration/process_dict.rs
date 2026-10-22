use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const PROCESS_DICT_PROBE: &[u8] =
    include_bytes!("../../fixtures/process_dict/process_dict_probe.beam");

/// Return the `~w` text of `expression` on the pinned ERTS.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(erts_result("process_dict_probe:large(10)").len() > 0, true);
/// ```
fn erts_result(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/process_dict",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    assert!(reference.stderr.is_empty(), "{reference:?}");
    String::from_utf8(reference.stdout).unwrap()
}

/// Return the `~w` text of a result that holds integers, atoms, tuples,
/// proper lists, and binaries.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(write(&program, &Term::tuple(vec![Term::Integer(1)])), "{1}");
/// ```
fn write(program: &BeamProgram, term: &Term) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| write(program, term))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Integer(value) => value.to_string(),
        Term::BigInt(value) => value.to_string(),
        Term::Atom(atom) => program.atoms().name_of(atom).to_owned(),
        Term::Tuple(elements) => format!("{{{}}}", join(elements)),
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
        _ => {
            let (elements, proper) = term.list_elements().unwrap();
            assert!(proper);
            format!("[{}]", join(&elements))
        }
    }
}

/// Run `function` of the probe with `arguments` on PBeam and on the pinned
/// ERTS, and assert the same `~w` text.
fn assert_erts_result(function: &str, arguments: &[i64]) {
    let text = arguments
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let expected = erts_result(&format!("process_dict_probe:{function}({text})"));
    let program = BeamProgram::load(&[PROCESS_DICT_PROBE]).unwrap();
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| program.host_term(Term::Integer(*argument)))
        .collect();
    let result = program
        .execute("process_dict_probe", function, &arguments)
        .unwrap()
        .term;
    assert_eq!(format!("{}\n", write(&program, &result)), expected);
}

#[test]
fn returns_a_small_dictionary_in_the_erts_order() {
    // `small/0` puts six static atoms and twelve keys of six shapes, so the
    // table grows past its 8 first buckets. It replaces one value with a
    // tuple and one with an integer, and erases a present and a missing key.
    // Each `get/0`, `get_keys/0,1`, and `erase/0` result has the ERTS order.
    assert_erts_result("small", &[]);
}

#[test]
fn returns_a_large_dictionary_in_the_erts_order_after_grows_and_shrinks() {
    // `large/1` grows the table to more than N buckets, replaces values,
    // erases every third key, shrinks the table with erases of present and
    // missing keys, and grows it again from a split position that is not 0.
    assert_erts_result("large", &[100]);
    assert_erts_result("large", &[2000]);
}
