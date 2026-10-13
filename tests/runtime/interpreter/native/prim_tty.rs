use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::term::Map;

#[test]
fn gives_ebadf_for_an_fd_that_is_not_open() {
    // `isatty(-1)` fails with `EBADF`, which is not `EINVAL` or
    // `ENOTTY` (prim_tty_nif.c:272-277). No fd layout of a child
    // process reaches this result for fds 0, 1, and 2, because Rust and
    // ERTS open `/dev/null` on a closed standard fd.
    let mut atoms = AtomTable::new();
    let result = isatty_fd(&mut atoms, -1).unwrap();
    assert!(
        matches!(&result, BifResult::Value(Term::Atom(atom)) if atom.name() == "ebadf"),
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
            &crate::term::test_atoms(),
        )
        .unwrap(),
    );
    let result = tty_init(&mut scheduler, &Term::Reference(reference), &options).unwrap();
    assert_eq!(
        format!("{result:?}"),
        r#"Value(Tuple([Atom("error"), Tuple([Atom("tcsetattr"), Atom("ebadf")])]))"#
    );
    let tty = scheduler
        .resources
        .borrow_mut::<Ttys>()
        .get(reference)
        .unwrap();
    assert_eq!((&tty.state, tty.owner), (&TtyState::Enabled, None));
}
