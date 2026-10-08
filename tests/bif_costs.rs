use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BIF_COST_PROBE: &[u8] = include_bytes!("../fixtures/preempt/bif_cost_probe.beam");

/// `bif_cost_probe:exits/0` on the pinned ERTS with `erl +S 1`. The exit of
/// `a` bumps 100 reductions (`erl_process.c:14229`). With 150 or fewer
/// reductions left (`K` 3846 and more), `a` switches out before its
/// monitors, so `b` sends 666 messages before the `DOWN` message. With 100
/// or fewer (`K` 3896 and more), it also switches out before it leaves the
/// process table, so `b` sees its name (`erl_process.c:14315-14316`, `14475`,
/// `14541-14543`).
const EXITS: &str = "[{3845,[{down,0,normal},{seen,false},{done,1000}]},\
                     {3846,[{seen,false},{down,666,normal},{done,334}]},\
                     {3895,[{seen,false},{down,666,normal},{done,334}]},\
                     {3896,[{seen,true},{down,666,normal},{done,334}]},\
                     {3990,[{seen,true},{down,666,normal},{done,334}]}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![Term::Integer(1)])), "[1]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn charges_the_process_exit_as_pinned_erts_with_one_scheduler() {
    let program = BeamProgram::load(&[BIF_COST_PROBE]).unwrap();
    let exits = program.execute("bif_cost_probe", "exits", &[]).unwrap();
    assert_eq!(text(&exits), EXITS);

    // The reference run checks that the pinned `erl` is the JIT flavor.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            "{module, M} = code:ensure_loaded(bif_cost_probe), \
             jit = erlang:system_info(emu_flavor), \
             io:format(\"~w~n\", [M:exits()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{EXITS}\n")
    );
}
