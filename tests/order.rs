use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const ORDER_PROBE: &[u8] = include_bytes!("../fixtures/order/order_probe.beam");

/// The `is_ge` result of each pair in `order_probe:main/0`: `ge` when the
/// left term is greater than or equal to the right term, else `lt`.
const RESULTS: &str = "lt,ge,lt,ge,ge,lt,ge,ge,ge,lt,ge,ge,ge,lt,lt,ge,ge,ge,lt,lt,lt,lt,ge,lt,ge,lt,ge,ge,lt,ge,ge,ge,lt,ge,ge,ge,ge";

#[test]
fn compares_terms_in_is_ge_as_pinned_erts() {
    // `main/0` runs the guard `A >= B` (`is_ge`) on pairs of each term type:
    // integers, atoms by name, references, pids by creation, tuples, flatmaps
    // and hashmaps, lists, and binaries. `pbeam_order_z` has a lower atom
    // index than `pbeam_order_a`, as `maps:keys/1` shows, but it is greater
    // in the term order.
    let program = BeamProgram::load(&[ORDER_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("order_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            Term::List(vec![atom("pbeam_order_z"), atom("pbeam_order_a")]),
            Term::List(RESULTS.split(',').map(atom).collect()),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/order",
            "-eval",
            "io:format(\"~w~n\", [order_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{{[pbeam_order_z,pbeam_order_a],[{RESULTS}]}}\n")
    );
}
