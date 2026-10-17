use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::term::{Map, debug_term};

const BADARG: &str = "Raise { class: Error, reason: Atom(\"badarg\") }";

/// Return the `Debug` text of a BIF result, with each atom name from `atoms`.
///
/// `Debug for Term` writes the atom index, because a bare term carries no
/// table. The atom names of a BIF result come from the run, so this test
/// reads them through the table of that run.
fn debug_bif(result: &BifResult, atoms: &AtomTable) -> String {
    match result {
        BifResult::Value(term) => format!("Value({})", debug_term(term, atoms)),
        BifResult::Raise { class, reason } => format!(
            "Raise {{ class: {class:?}, reason: {} }}",
            debug_term(reason, atoms)
        ),
        other => format!("{other:?}"),
    }
}

#[test]
fn gives_ebadf_for_an_fd_that_is_not_open() {
    // `isatty(-1)` fails with `EBADF`, which is not `EINVAL` or
    // `ENOTTY` (prim_tty_nif.c:272-277). No fd layout of a child
    // process reaches this result for fds 0, 1, and 2, because Rust and
    // ERTS open `/dev/null` on a closed standard fd.
    let atoms = AtomTable::new();
    let result = isatty_fd(-1);
    assert!(
        matches!(&result, BifResult::Value(Term::Atom(atom)) if atoms.name_of(atom) == "ebadf"),
        "{result:?}"
    );
}

#[test]
fn gives_the_tcsetattr_errno_and_keeps_no_owner() {
    // `tcsetattr` on an fd that is not open fails with `EBADF`, and
    // `tty_init/2` returns before it sets `self` and monitors the caller
    // (prim_tty_nif.c:1131-1133). No fd layout reaches this result: a
    // TTY resource is `disabled` only when `tcgetattr` of its output fd
    // succeeds, and `tcsetattr` of the same fd then succeeds.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    let reference = scheduler.make_ref().unwrap();
    scheduler.resources.borrow_mut::<Ttys>().insert(
        reference,
        TtyResource {
            ofd: -1,
            state: TtyState::Disabled,
            first_mode: terminal::EMPTY_MODE,
            set_mode: terminal::EMPTY_MODE,
            owner: None,
            reader: None,
        },
    );
    let options = Term::Map(
        Map::from_pairs(
            vec![(Term::static_atom("input"), Term::static_atom("raw"))],
            &crate::term::test_atom_table(),
        )
        .unwrap(),
    );
    let result = tty_init(&mut scheduler, &Term::Reference(reference), &options).unwrap();
    assert_eq!(
        debug_bif(&result, &scheduler.atoms),
        r#"Value(Tuple([Atom("error"), Tuple([Atom("tcsetattr"), Atom("ebadf")])]))"#
    );
    let tty = scheduler
        .resources
        .borrow_mut::<Ttys>()
        .get(reference)
        .unwrap();
    assert_eq!((&tty.state, tty.owner), (&TtyState::Enabled, None));
}

#[test]
fn raises_badarg_for_the_termcap_and_width_arguments_of_erts() {
    // `tigetstr_nif/1` and `tputs_nif/2` inspect the iodata code with
    // `enif_inspect_iolist_as_binary`, and `wcwidth/1` gets a C `int` with
    // `enif_get_int` (prim_tty_nif.c:309-325, 865-881, 922-945). Each
    // failure raises `badarg` before a curses call.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    let bad_argument = Term::list_from(vec![Term::Integer(256)]);
    for code in [
        bad_argument.clone(),
        Term::tuple(vec![]),
        Term::Float(1.0),
        Term::Reference(scheduler.make_ref().unwrap()),
    ] {
        assert_eq!(
            debug_bif(
                &tigetstr_nif(&mut scheduler, &code).unwrap(),
                &scheduler.atoms
            ),
            BADARG
        );
        assert_eq!(
            debug_bif(
                &tputs_nif(&mut scheduler, &code, &Term::nil()).unwrap(),
                &scheduler.atoms
            ),
            BADARG
        );
    }
    // The `Args` of `tputs_nif/2` takes integers, and `wcwidth/1` takes one.
    let code = Term::binary(b"cup\0".to_vec());
    for arguments in [
        Term::list_from(vec![Term::tuple(vec![])]),
        Term::list_from(vec![Term::Float(1.0)]),
    ] {
        assert_eq!(
            debug_bif(
                &tputs_nif(&mut scheduler, &code, &arguments).unwrap(),
                &scheduler.atoms
            ),
            BADARG
        );
    }
    for character in [
        Term::tuple(vec![]),
        Term::Float(1.0),
        Term::Integer(1 << 40),
    ] {
        assert_eq!(
            debug_bif(
                &wcwidth(&mut scheduler, &character).unwrap(),
                &scheduler.atoms
            ),
            BADARG
        );
    }
}

#[test]
fn gives_the_three_capability_lists_of_ncurses() {
    // `tty_tinfo_nif` makes the map `#{bool => _, num => _, str => _}`,
    // and each list holds a map with the `name`, `code`, and `full_name`
    // strings of one ncurses capability (prim_tty_nif.c:794-839). The
    // integration test compares the three lists with the pinned ERTS, so
    // this test checks their shapes only.
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    let BifResult::Value(term) = tinfo_nif(&mut scheduler).unwrap() else {
        panic!("tinfo_nif gives a value");
    };
    let Term::Map(map) = &term else {
        panic!("tinfo_nif gives a map");
    };
    let mut key = |name: &str| Term::Atom(scheduler.atoms.put(name).unwrap());
    let lengths: Vec<usize> = ["bool", "num", "str"]
        .map(|name| {
            let entries = map.get(&key(name)).unwrap();
            assert!(entries.is_list(), "the {name} value is a list");
            let elements = entries.list_iter().expect("a list iterates");
            elements.count()
        })
        .into();
    assert!(lengths[0] > 0 && lengths[1] > 0);
    assert!(lengths[2] > 400);
    let Term::Map(capability) = map
        .get(&key("str"))
        .unwrap()
        .list_head()
        .expect("the str list has entries")
    else {
        panic!("an entry is a map");
    };
    for name in ["name", "code", "full_name"] {
        let value = capability.get(&key(name)).unwrap();
        assert!(value.is_list(), "the {name} value is a string");
    }
}

#[test]
fn gives_the_tparm_error_of_a_failed_expansion() {
    // `tty_tputs_nif` gives `{error, {tparm, Errno}}` when `tparm` fails
    // (prim_tty_nif.c:959-962). A failed `tparm` needs an allocation
    // failure, so the test drives `tputs_result` with the failure directly.
    let mut atoms = AtomTable::new();
    let error = tputs_result(&mut atoms, Err(libc::ENOMEM)).unwrap();
    assert_eq!(
        debug_bif(&error, &atoms),
        r#"Value(Tuple([Atom("error"), Tuple([Atom("tparm"), Atom("enomem")])]))"#
    );
    let ok = tputs_result(&mut atoms, Ok(b"7".to_vec())).unwrap();
    assert_eq!(
        debug_bif(&ok, &atoms),
        r#"Value(Tuple([Atom("ok"), Binary([55])]))"#
    );
}
