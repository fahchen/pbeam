//! Unit tests of the `asn1rt_nif` NIFs for the arguments that no fixture
//! can give: PBeam builds no bitstring that is not a whole number of bytes.

use super::*;
use crate::runtime::capability::{AllowAll, Policy};
use crate::runtime::process::tests::frame;
use crate::term::Binary;

/// Return a scheduler whose current process is a new root process.
fn scheduler() -> Scheduler {
    let mut scheduler = Scheduler::new(AtomTable::new());
    scheduler
        .start(frame(), vec![], AllowAll.initial())
        .unwrap();
    scheduler.schedule_next().unwrap();
    scheduler
}

/// Return the reason of an `error` exception, or panic.
fn reason(result: BifResult) -> Term {
    match result {
        BifResult::Raise {
            class: ExceptionClass::Error,
            reason,
        } => reason,
        other => panic!("the result is an error exception, not {other:?}"),
    }
}

/// Return the value of a result, or panic.
fn value(result: BifResult) -> Term {
    match result {
        BifResult::Value(term) => term,
        other => panic!("the result is a value, not {other:?}"),
    }
}

#[test]
fn rejects_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `enif_inspect_iolist_as_binary` accepts no such bitstring, so the
    // two iodata NIFs raise `badarg` (asn1_erl_nif.c:1208-1209,
    // 1265-1266), and `enif_inspect_binary` fails in `ber_encode`, so
    // `encode_ber_tlv` gives `{error, -1}` (asn1_erl_nif.c:1041-1042). The
    // pinned ERTS gives the same results.
    let mut scheduler = scheduler();
    let bits = Term::Binary(Binary::from(&[5u8, 1][..]).bits(0, 9));
    let iodata = Term::list_from(vec![Term::binary(vec![1]), bits.clone()]);
    let result = encode_per_complete(&mut scheduler, &iodata).unwrap();
    assert_eq!(reason(result), Term::static_atom("badarg"));
    let result = decode_ber_tlv_raw(&mut scheduler, &bits).unwrap();
    assert_eq!(reason(result), Term::static_atom("badarg"));
    let error = Term::tuple(vec![Term::static_atom("error"), Term::Integer(-1)]);
    for tlv in [
        Term::tuple(vec![Term::Integer(2), bits.clone()]),
        Term::tuple(vec![
            Term::Integer(16),
            Term::list_from(vec![Term::tuple(vec![Term::Integer(2), bits.clone()])]),
        ]),
    ] {
        let result = encode_ber_tlv(&mut scheduler, &tlv).unwrap();
        assert_eq!(value(result), error);
    }
}
