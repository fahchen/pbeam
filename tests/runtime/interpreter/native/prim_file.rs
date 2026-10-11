use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

#[test]
fn checks_the_follow_flag_and_then_the_path_as_erts() {
    // `prim_file` does not export `read_info_nif/2`, and its only caller,
    // `read_info_1/3`, gives the bytes of `internal_name2native/1` and the
    // flag 0 or 1 (prim_file.erl:657-664). Thus no BEAM code reaches these
    // checks of `read_info_nif` (prim_file_nif.c:1019-1038): `enif_get_int`
    // first, then `efile_marshal_path` with `has_invalid_null_termination`
    // (unix_prim_file.c:71-101).
    let mut scheduler = scheduler();
    let path = Term::Binary(b"/\0".to_vec());
    for (path, follow, expected) in [
        (
            path.clone(),
            Term::Integer(1 << 31),
            "Raise { class: Error, reason: Atom(\"badarg\") }",
        ),
        (
            path.clone(),
            Term::Integer(-(1 << 31) - 1),
            "Raise { class: Error, reason: Atom(\"badarg\") }",
        ),
        (
            Term::Integer(1),
            Term::static_atom("true"),
            "Raise { class: Error, reason: Atom(\"badarg\") }",
        ),
        (
            Term::list_from(vec![Term::Integer(47), Term::Integer(0)]),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::Binary(vec![]),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::Binary(b"/".to_vec()),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::Binary(b"/\0a\0".to_vec()),
            Term::Integer(1),
            "Value(Tuple([Atom(\"error\"), Atom(\"einval\")]))",
        ),
        (
            Term::Binary(b"/pbeam-missing\0\0".to_vec()),
            Term::Integer((1 << 31) - 1),
            "Value(Tuple([Atom(\"error\"), Atom(\"enoent\")]))",
        ),
        (
            Term::Binary(b"\0".to_vec()),
            Term::Integer(-(1 << 31)),
            "Value(Tuple([Atom(\"error\"), Atom(\"enoent\")]))",
        ),
    ] {
        let result = read_info_nif(&mut scheduler, &path, &follow).unwrap();
        assert_eq!(format!("{result:?}"), expected, "{path:?} {follow:?}");
    }
    let result = read_info_nif(&mut scheduler, &path, &Term::Integer(0)).unwrap();
    assert!(
        matches!(&result, BifResult::Value(Term::Tuple(fields)) if fields.len() == 14
            && matches!(&fields[2], Term::Atom(atom) if atom.name() == "directory")),
        "{result:?}"
    );
}
