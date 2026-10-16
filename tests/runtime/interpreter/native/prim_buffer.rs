use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::interpreter::native::call;
use crate::runtime::process::tests::frame;
use crate::term::Reference;
use num_bigint::BigInt;
use std::sync::Arc;

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

/// Return the term of a new buffer resource.
fn buffer(scheduler: &mut Scheduler) -> Term {
    match new_nif(scheduler).unwrap() {
        BifResult::Value(term) => term,
        other => panic!("new_nif gives a value, not {other:?}"),
    }
}

/// Write the binaries `binaries` to `buffer` and assert the result `ok`.
fn write(scheduler: &mut Scheduler, buffer: &Term, binaries: &[&[u8]]) {
    let iovec = Term::list_from(binaries.iter().map(|bytes| Term::binary(bytes.to_vec())));
    let result = write_nif(scheduler, buffer, &iovec).unwrap();
    assert!(matches!(result, BifResult::Value(_)), "{result:?}");
}

/// Return the `size/1` of `buffer`.
fn size(scheduler: &mut Scheduler, buffer: &Term) -> i64 {
    match size_nif(scheduler, buffer).unwrap() {
        BifResult::Value(Term::Integer(bytes)) => bytes,
        other => panic!("size/1 gives an integer, not {other:?}"),
    }
}

/// Return the name of the atom result of `result`.
fn atom_name(scheduler: &Scheduler, result: &BifResult) -> String {
    match result {
        BifResult::Value(Term::Atom(atom)) => scheduler.atoms.name_of(atom).to_owned(),
        other => panic!("the result is an atom, not {other:?}"),
    }
}

#[test]
fn calls_each_binding_of_the_library_through_the_registry() {
    // The `nif_start` instruction finds each function of `nif_funcs` by its
    // name and arity (prim_buffer_nif.c:86-96).
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    let iovec = Term::list_from(vec![Term::binary(b"ab".to_vec())]);
    let calls = [
        ("new", vec![]),
        ("size", vec![buffer.clone()]),
        ("write", vec![buffer.clone(), iovec]),
        ("peek_head", vec![buffer.clone()]),
        ("copying_read", vec![buffer.clone(), Term::Integer(1)]),
        ("skip", vec![buffer.clone(), Term::Integer(1)]),
        ("find_byte_index", vec![buffer.clone(), Term::Integer(10)]),
        ("try_lock", vec![buffer.clone()]),
        ("unlock", vec![buffer.clone()]),
    ];
    for (name, arguments) in calls {
        let result = call(&mut scheduler, "prim_buffer", name, &arguments);
        assert!(
            matches!(result, Some(Ok(BifResult::Value(_)))),
            "{name} gives a value, not {result:?}"
        );
    }
}

#[test]
fn gives_a_new_buffer_resource() {
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    assert!(matches!(buffer, Term::Reference(_)));
    assert_eq!(size(&mut scheduler, &buffer), 0);
}

#[test]
fn raises_badarg_for_a_term_that_is_not_a_buffer_resource() {
    let mut scheduler = scheduler();
    let reference = Term::Reference(Reference(7));
    let iovec = Term::nil();
    for (name, arguments) in [
        ("size", vec![Term::nil()]),
        ("peek_head", vec![reference.clone()]),
        ("copying_read", vec![Term::nil(), Term::Integer(1)]),
        ("write", vec![reference.clone(), iovec.clone()]),
        ("skip", vec![Term::nil(), Term::Integer(1)]),
        (
            "find_byte_index",
            vec![reference.clone(), Term::Integer(10)],
        ),
        ("try_lock", vec![Term::nil()]),
        ("unlock", vec![reference.clone()]),
    ] {
        let result = call(&mut scheduler, "prim_buffer", name, &arguments);
        assert!(
            matches!(
                result,
                Some(Ok(BifResult::Raise {
                    class: ExceptionClass::Error,
                    ..
                }))
            ),
            "{name} raises badarg, not {result:?}"
        );
    }
}

#[test]
fn reads_writes_and_searches_the_buffer_as_the_nifs_do() {
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    write(&mut scheduler, &buffer, &[b"ab\ncd\n"]);
    assert_eq!(size(&mut scheduler, &buffer), 6);
    // The head is the binary at the front of the buffer, with no change.
    let head = peek_head_nif(&mut scheduler, &buffer).unwrap();
    assert!(matches!(head, BifResult::Value(Term::Binary(_))));
    assert_eq!(size(&mut scheduler, &buffer), 6);
    // The copy removes the bytes, and a larger copy raises badarg.
    assert!(matches!(
        copying_read_nif(&mut scheduler, &buffer, &Term::Integer(3)).unwrap(),
        BifResult::Value(Term::Binary(_))
    ));
    assert_eq!(size(&mut scheduler, &buffer), 3);
    assert!(matches!(
        copying_read_nif(&mut scheduler, &buffer, &Term::Integer(4)).unwrap(),
        BifResult::Raise { .. }
    ));
    assert_eq!(size(&mut scheduler, &buffer), 3);
    // The search gives the index from the front of the buffer.
    let found = find_byte_index_nif(&mut scheduler, &buffer, &Term::Integer(b'\n'.into())).unwrap();
    assert!(
        matches!(found, BifResult::Value(Term::Tuple(ref fields)) if fields.len() == 2),
        "{found:?}"
    );
    let missing =
        find_byte_index_nif(&mut scheduler, &buffer, &Term::Integer(b'x'.into())).unwrap();
    assert_eq!(atom_name(&scheduler, &missing), "not_found");
    // The skip removes bytes, and a larger skip raises badarg.
    let skipped = skip_nif(&mut scheduler, &buffer, &Term::Integer(3)).unwrap();
    assert_eq!(atom_name(&scheduler, &skipped), "ok");
    assert_eq!(size(&mut scheduler, &buffer), 0);
    assert!(matches!(
        skip_nif(&mut scheduler, &buffer, &Term::Integer(1)).unwrap(),
        BifResult::Raise { .. }
    ));
    // An empty buffer has no head.
    assert!(matches!(
        peek_head_nif(&mut scheduler, &buffer).unwrap(),
        BifResult::Raise { .. }
    ));
}

#[test]
fn checks_the_size_argument_of_the_reads_and_of_the_skip() {
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    let bad_sizes = [
        Term::Integer(-1),
        Term::Float(1.0),
        Term::static_atom("ok"),
        Term::BigInt(Arc::new(BigInt::from(1) << 64)),
    ];
    for size in bad_sizes {
        for result in [
            copying_read_nif(&mut scheduler, &buffer, &size).unwrap(),
            skip_nif(&mut scheduler, &buffer, &size).unwrap(),
        ] {
            assert!(matches!(result, BifResult::Raise { .. }), "{result:?}");
        }
    }
    // `enif_get_uint64` accepts a bignum in the `u64` range.
    assert_eq!(
        unsigned(&Term::BigInt(Arc::new(BigInt::from(1) << 63))),
        Some(1 << 63)
    );
    assert_eq!(unsigned(&Term::Integer(0)), Some(0));
}

#[test]
fn checks_the_needle_of_find_byte_index() {
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    for needle in [
        Term::Float(10.0),
        Term::static_atom("ok"),
        Term::Integer(-1),
        Term::Integer(256),
        Term::BigInt(Arc::new(BigInt::from(1) << 40)),
    ] {
        let result = find_byte_index_nif(&mut scheduler, &buffer, &needle).unwrap();
        assert!(matches!(result, BifResult::Raise { .. }), "{result:?}");
    }
    // The range check of the needle comes after the flush of the
    // accumulator (prim_buffer_nif.c:432-436).
    write(&mut scheduler, &buffer, &[b"ab"]);
    assert!(matches!(
        find_byte_index_nif(&mut scheduler, &buffer, &Term::Integer(256)).unwrap(),
        BifResult::Raise { .. }
    ));
    assert_eq!(size(&mut scheduler, &buffer), 2);
    assert!(matches!(
        peek_head_nif(&mut scheduler, &buffer).unwrap(),
        BifResult::Value(Term::Binary(_))
    ));
}

#[test]
fn takes_and_releases_the_lock_as_the_nifs_do() {
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    let acquired = try_lock_nif(&mut scheduler, &buffer).unwrap();
    assert_eq!(atom_name(&scheduler, &acquired), "acquired");
    let busy = try_lock_nif(&mut scheduler, &buffer).unwrap();
    assert_eq!(atom_name(&scheduler, &busy), "busy");
    let released = unlock_nif(&mut scheduler, &buffer).unwrap();
    assert_eq!(atom_name(&scheduler, &released), "ok");
    let violation = unlock_nif(&mut scheduler, &buffer).unwrap();
    assert!(
        matches!(
            violation,
            BifResult::Raise {
                class: ExceptionClass::Error,
                reason: Term::Atom(ref atom),
            } if scheduler.atoms.name_of(&atom) == "lock_order_violation"
        ),
        "{violation:?}"
    );
}

#[test]
fn writes_an_iovec_in_calls_of_at_most_the_first_64_elements() {
    let mut scheduler = scheduler();
    let buffer = buffer(&mut scheduler);
    // ERTS runs `write/2` again with the tail of an iovec of more than 64
    // elements (prim_buffer_nif.c:357-361).
    let iovec = Term::list_from((0..70).map(|_| Term::binary(b"x".to_vec())));
    assert!(matches!(
        write_nif(&mut scheduler, &buffer, &iovec).unwrap(),
        BifResult::Value(_)
    ));
    assert_eq!(size(&mut scheduler, &buffer), 70);
    // ERTS writes the first 64 elements and raises badarg for the element
    // that is not a binary in the next call.
    let mut elements: Vec<Term> = (0..64).map(|_| Term::binary(b"y".to_vec())).collect();
    elements.push(Term::static_atom("ok"));
    let iovec = Term::list_from(elements);
    assert!(matches!(
        write_nif(&mut scheduler, &buffer, &iovec).unwrap(),
        BifResult::Raise { .. }
    ));
    assert_eq!(size(&mut scheduler, &buffer), 134);
    // A term that is not a list of binaries raises badarg, also the tail of
    // an improper list (erl_nif.c:4107-4166).
    let iovec = Term::list_with_tail(vec![Term::binary(b"z".to_vec())], Term::static_atom("ok"));
    assert!(matches!(
        write_nif(&mut scheduler, &buffer, &iovec).unwrap(),
        BifResult::Raise { .. }
    ));
    assert!(matches!(
        write_nif(&mut scheduler, &buffer, &Term::binary(b"z".to_vec())).unwrap(),
        BifResult::Raise { .. }
    ));
    assert_eq!(size(&mut scheduler, &buffer), 134);
    // A write of no bytes and of empty binaries gives ok and no bytes.
    write(&mut scheduler, &buffer, &[]);
    write(&mut scheduler, &buffer, &[b"", b""]);
    assert_eq!(size(&mut scheduler, &buffer), 134);
}
