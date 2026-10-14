//! The result of a comparison of an atom name with a static name in each
//! changed group, on PBeam and on the pinned ERTS. The probe compares a
//! table BIF, a process flag, and a binary split option, so it covers the
//! comparison forms that changed in task #594.

use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const STATIC_ATOM_PROBE: &[u8] =
    include_bytes!("../../fixtures/static_atom/static_atom_probe.beam");

/// The `~w` text of `static_atom_probe:main()` on the pinned ERTS.
const ERTS_RESULT: &[u8] = b"[true,true,[<<>>,<<2,1,2>>],[<<1,1>>],true,off_heap]\n";

#[test]
fn compares_a_static_atom_name_as_pinned_erts() {
    // `static_atom_probe:main/0` calls `erts_internal:garbage_collect/1`
    // with `minor` and `major`, `binary:split/3` with `trim` and `trim_all`,
    // and `erlang:process_flag/2` with `trap_exit` and
    // `message_queue_data`. Each call compares the name of an atom with a
    // static name. The probe sets each flag before it reads it, so the
    // result does not depend on the initial flag.
    let program = BeamProgram::load(&[STATIC_ATOM_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    assert_eq!(
        program
            .execute("static_atom_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            atom("true"),
            atom("true"),
            Term::list_from(vec![binary(b""), binary(&[2, 1, 2])]),
            Term::list_from(vec![binary(&[1, 1])]),
            atom("true"),
            atom("off_heap"),
        ])
    );

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/static_atom",
            "-eval",
            "io:format(\"~w~n\", [static_atom_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&reference.stdout),
        String::from_utf8_lossy(ERTS_RESULT)
    );
}
