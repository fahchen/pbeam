use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const GROUP_PROBE: &[u8] = include_bytes!("../fixtures/group/group_probe.beam");

#[test]
fn takes_the_fail_labels_of_guard_bifs_and_is_ge_as_pinned_erts() {
    // `kind/1` runs `tuple_size/1` and `element/2` through `bif` instructions
    // with a fail label that is not 0, and `is_ge` on two integers. `atom`
    // makes `tuple_size/1` jump to the fail label, and `{x}` makes `is_ge`
    // jump.
    let program = BeamProgram::load(&[GROUP_PROBE]).unwrap();
    let output = Term::Atom(program.atom("output").unwrap());
    let other = Term::Atom(program.atom("other").unwrap());
    assert_eq!(
        program.execute("group_probe", "guards", &[]).unwrap(),
        Term::List(vec![output, other.clone(), other])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/group",
            "-eval",
            "io:format(\"~p~n\", [group_probe:guards()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(reference.stdout, b"[output,other,other]\n");
}

#[test]
fn registers_a_process_name_as_pinned_erts() {
    // `registry/0` registers `undefined`, a first name, a second name for the
    // same process, and a name that another process holds, then reads the
    // `registered_name` of the caller.
    let program = BeamProgram::load(&[GROUP_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("group_probe", "registry", &[]).unwrap(),
        Term::Tuple(vec![
            atom("badarg"),
            atom("true"),
            atom("badarg"),
            atom("badarg"),
            Term::Tuple(vec![atom("registered_name"), atom("pbeam_probe_reg")]),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/group",
            "-eval",
            "io:format(\"~p~n\", [group_probe:registry()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    assert_eq!(
        reference.stdout,
        b"{badarg,true,badarg,badarg,{registered_name,pbeam_probe_reg}}\n"
    );
}
