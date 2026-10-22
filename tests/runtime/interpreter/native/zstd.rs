//! Unit tests of the `zstd` stream NIFs for the arguments that no fixture
//! can give: `zstd.erl` passes only the end directives 0, 1, and 2
//! (`zstd.erl:246-248`), and PBeam builds no bitstring that is not a whole
//! number of bytes.

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

/// Return the value of a result, or panic.
fn value(result: BifResult) -> Term {
    match result {
        BifResult::Value(term) => term,
        other => panic!("the result is a value, not {other:?}"),
    }
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

#[test]
fn raises_badarg_for_a_flush_that_is_not_a_c_int() {
    // `codec_nif` reads the flush with `enif_get_int` before the input
    // (zstd_nif.c:599-601).
    let mut scheduler = scheduler();
    let compress = value(init_compress_nif(&mut scheduler).unwrap());
    let decompress = value(init_decompress_nif(&mut scheduler).unwrap());
    let data = Term::binary(b"a".to_vec());
    for flush in [Term::Nil, Term::Integer(1 << 40)] {
        let result = compress_stream_nif(&mut scheduler, &compress, &data, &flush).unwrap();
        assert_eq!(reason(result), Term::static_atom("badarg"));
        let result = decompress_stream_nif(&mut scheduler, &decompress, &data, &flush).unwrap();
        assert_eq!(reason(result), Term::static_atom("badarg"));
    }
}

#[test]
fn raises_the_out_of_bound_error_for_an_end_directive_above_the_end() {
    // `ZSTD_compressStream2` rejects an end directive above `ZSTD_e_end`
    // as an unsigned value (zstd_compress.c:6456).
    let mut scheduler = scheduler();
    let compress = value(init_compress_nif(&mut scheduler).unwrap());
    let data = Term::binary(b"a".to_vec());
    for flush in [3, -1] {
        let result =
            compress_stream_nif(&mut scheduler, &compress, &data, &Term::Integer(flush)).unwrap();
        let expected = Term::tuple(vec![
            Term::Atom(scheduler.atoms.put("zstd_error").unwrap()),
            Term::binary(b"Parameter is out of bound".to_vec()),
        ]);
        assert_eq!(reason(result), expected);
    }
}

#[test]
fn raises_badarg_for_a_bitstring_that_is_not_a_whole_number_of_bytes() {
    // `enif_inspect_binary` accepts no such bitstring (zstd_nif.c:603-605,
    // 743-745).
    let mut scheduler = scheduler();
    let compress = value(init_compress_nif(&mut scheduler).unwrap());
    let decompress = value(init_decompress_nif(&mut scheduler).unwrap());
    let data = Term::Binary(Binary::from(b"ab").bits(0, 9));
    let flush = Term::Integer(0);
    let result = compress_stream_nif(&mut scheduler, &compress, &data, &flush).unwrap();
    assert_eq!(reason(result), Term::static_atom("badarg"));
    let result = decompress_stream_nif(&mut scheduler, &decompress, &data, &flush).unwrap();
    assert_eq!(reason(result), Term::static_atom("badarg"));
}

#[test]
fn gives_done_for_an_empty_binary_with_the_flush_atom_true_before_a_frame() {
    // The first call of a frame gives `{done, <<>>}` for an empty binary
    // and the flush atom `true`, before the flush check, and does not start
    // the context (zstd_nif.c:747-753).
    let mut scheduler = scheduler();
    let decompress = value(init_decompress_nif(&mut scheduler).unwrap());
    let flush = Term::Atom(scheduler.atoms.put("true").unwrap());
    let empty = Term::binary(Vec::new());
    let result = decompress_stream_nif(&mut scheduler, &decompress, &empty, &flush).unwrap();
    let done = Term::Atom(scheduler.atoms.put("done").unwrap());
    assert_eq!(value(result), Term::tuple(vec![done, empty.clone()]));
    // A binary that is not empty reads the flush as an `int`.
    let data = Term::binary(b"a".to_vec());
    let result = decompress_stream_nif(&mut scheduler, &decompress, &data, &flush).unwrap();
    assert_eq!(reason(result), Term::static_atom("badarg"));
}
