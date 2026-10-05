use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const IMPROPER_PROBE: &[u8] = include_bytes!("../fixtures/improper/improper_probe.beam");

/// Run a function of `improper_probe` under the pinned ERTS and return the
/// `~w` text of its result.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(erl_reference("literals").starts_with("{[a|b],"));
/// ```
fn erl_reference(function: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/improper",
            "-eval",
            &format!("io:format(\"~w~n\", [improper_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    String::from_utf8(reference.stdout).unwrap()
}

/// Return the list of the integers.
fn integers(elements: &[i64]) -> Vec<Term> {
    elements
        .iter()
        .map(|element| Term::Integer(*element))
        .collect()
}

/// Return an improper list of elements and a tail.
fn improper(elements: Vec<Term>, tail: Term) -> Term {
    Term::ImproperList(elements, Box::new(tail))
}

#[test]
fn loads_compares_and_hashes_improper_list_literals_as_pinned_erts() {
    // `literals/0` loads improper list literals (a `LIST_EXT` whose tail is
    // not `NIL_EXT`), also in a tuple and as map keys. It compares pairs
    // with `is_ge`, `is_lt`, `erlang:'=:='/2`, and `erlang:'>='/2`
    // (`eq`, `utils.c:1137-1162`; `erts_cmp_compound`, `utils.c:1827-1851`),
    // and reads the flatmap key order and the hashmap order
    // (`make_internal_hash`, `erl_term_hashing.c:1783-1841`).
    let program = BeamProgram::load(&[IMPROPER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let atoms = |names: &str| Term::List(names.split(',').map(atom).collect());
    let int = Term::Integer;
    let one_two = || improper(vec![int(1)], int(2));
    let nested = || improper(vec![one_two()], int(3));
    let hash_keys = [
        int(18),
        improper(vec![Term::List(vec![int(1)])], int(2)),
        nested(),
        int(4),
        improper(vec![int(256)], int(1)),
    ]
    .into_iter()
    .chain(integers(&[12, 19, 13, 2, 7, 8, 10, 23, 9]))
    .chain([
        improper(vec![atom("true")], atom("false")),
        one_two(),
        int(15),
    ])
    .chain([improper(integers(&[1, 2]), int(3))])
    .chain(integers(&[1, 25, 20, 6, 11, 17, 24, 14]))
    .chain([improper(vec![Term::List(vec![])], int(-1))])
    .chain(integers(&[3, 16, 21, 5, 26, 22]))
    .collect();
    assert_eq!(
        program.execute("improper_probe", "literals", &[]).unwrap(),
        Term::Tuple(vec![
            improper(vec![atom("a")], atom("b")),
            Term::Tuple(vec![improper(
                integers(&[1, 2]),
                Term::Tuple(vec![atom("t")])
            )]),
            atoms("lt,ge,lt,lt,lt,lt,lt,ge,lt"),
            atoms("lt,ge,lt,lt,lt,lt,lt,ge,lt"),
            atoms("false,false,false,false,false,false,false,true,false"),
            atoms("false,true,false,false,false,false,false,true,false"),
            Term::List(vec![
                atom("x"),
                improper(vec![int(1)], atom("x")),
                Term::List(vec![int(1)]),
                nested(),
            ]),
            atom("d"),
            Term::List(hash_keys),
            atom("v"),
        ])
    );
    assert_eq!(
        erl_reference("literals"),
        "{[a|b],{[1,2|{t}]},[lt,ge,lt,lt,lt,lt,lt,ge,lt],[lt,ge,lt,lt,lt,lt,lt,ge,lt],\
         [false,false,false,false,false,false,false,true,false],\
         [false,true,false,false,false,false,false,true,false],[x,[1|x],[1],[[1|2]|3]],d,\
         [18,[[1]|2],[[1|2]|3],4,[256|1],12,19,13,2,7,8,10,23,9,[true|false],[1|2],15,\
         [1,2|3],1,25,20,6,11,17,24,14,[[]|-1],3,16,21,5,26,22],v}\n"
    );
}
