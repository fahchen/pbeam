use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const ETS_PROBE: &[u8] = include_bytes!("../fixtures/ets/ets_probe.beam");

/// Run `ets_probe:Function()` under the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/ets",
            "-eval",
            &format!("io:format(\"~w~n\", [ets_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    output.stdout
}

#[test]
fn makes_tables_and_raises_badarg_as_pinned_erts() {
    // `ets_probe:new/0` makes an unnamed table, a named table, and a table
    // with each option that ERTS accepts (erl_db.c:2597-2710). A heir that
    // is not alive acts as `none` (erl_db.c:5319-5337). Each case of the
    // last list raises `badarg`: a name that is not an atom, options that
    // are not a list, an option that is not valid, `ordered_set` with an
    // explicit lock count (erl_db.c:2717-2758), and a name in use
    // (erl_db.c:2821-2831), also with the type `bag`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = || Term::Tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program.execute("ets_probe", "new", &[]).unwrap(),
        Term::List(vec![
            atom("reference"),
            atom("ets_probe_named"),
            badarg(),
            atom("reference"),
            atom("ets_probe"),
            atom("reference"),
            Term::List(vec![badarg(); 19]),
        ])
    );
    assert_eq!(
        reference("new"),
        b"[reference,ets_probe_named,{error,badarg},reference,ets_probe,reference,[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]]\n"
    );
}

#[test]
fn deletes_the_tables_of_an_owner_before_its_down_message_as_pinned_erts() {
    // `ets_probe:owner_exit/0` cannot make a table with the name of a table
    // that another process owns. After the `DOWN` message of the owner, the
    // name is free, because ERTS deletes the tables of an exiting process
    // before its monitors fire (erl_process.c:14360-14368).
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("ets_probe", "owner_exit", &[]).unwrap(),
        Term::List(vec![
            Term::Tuple(vec![atom("error"), atom("badarg")]),
            atom("stopped"),
            atom("ets_probe_owned"),
        ])
    );
    assert_eq!(
        reference("owner_exit"),
        b"[{error,badarg},stopped,ets_probe_owned]\n"
    );
}

#[test]
fn rejects_tables_that_pbeam_cannot_model() {
    // ERTS 29.1.1 makes each of these tables: the types bag, bag,
    // duplicate_bag, and ordered_set, and two set tables whose heir is the
    // caller. PBeam supports only set tables without a live heir. This is
    // the "ETS tables" row in `docs/erts-differences.md`.
    let program = BeamProgram::load(&[ETS_PROBE]).unwrap();
    for (case, detail) in [
        ("bag", "table type bag"),
        ("set_after_bag", "table type bag"),
        ("duplicate_bag", "table type duplicate_bag"),
        ("ordered_set", "table type ordered_set"),
        ("heir", "heir ProcessId(0)"),
        ("heir_data", "heir ProcessId(0)"),
    ] {
        let message = format!("unsupported ets:new/2 {detail}");
        let case = Term::Atom(program.atom(case).unwrap());
        let error = program
            .execute("ets_probe", "unsupported", &[case])
            .unwrap_err();
        assert!(
            error.to_string().contains(&message),
            "expected {message}, got {error}"
        );
    }
}
