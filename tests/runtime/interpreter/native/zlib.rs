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

/// Return the term of a new zlib stream.
fn stream(scheduler: &mut Scheduler) -> Term {
    match open_nif(scheduler).unwrap() {
        BifResult::Value(term) => term,
        other => panic!("open_nif gives a value, not {other:?}"),
    }
}

/// Start a deflate stream with level 6, method deflated, window bits 15,
/// memory level 8, and the default strategy.
fn start_deflate(scheduler: &mut Scheduler, stream: &Term) {
    let result = deflate_init_nif(
        scheduler,
        stream,
        &Term::Integer(6),
        &Term::Integer(8),
        &Term::Integer(15),
        &Term::Integer(8),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(atom_name(scheduler, result), "ok");
}

/// Start an inflate stream with window bits 15 and the `cut` behavior.
fn start_inflate(scheduler: &mut Scheduler, stream: &Term) {
    let result =
        inflate_init_nif(scheduler, stream, &Term::Integer(15), &Term::Integer(2)).unwrap();
    assert_eq!(atom_name(scheduler, result), "ok");
}

/// Return the name of the atom result of `result`, or panic.
fn atom_name(scheduler: &Scheduler, result: BifResult) -> String {
    match &result {
        BifResult::Value(Term::Atom(atom)) => scheduler.atoms.name_of(atom).to_owned(),
        _ => panic!("the result is an atom, not {result:?}"),
    }
}

/// Return the atom reason of a raise.
fn raise_name(scheduler: &Scheduler, result: BifResult) -> String {
    match &result {
        BifResult::Raise {
            class: ExceptionClass::Error,
            reason: Term::Atom(atom),
        } => scheduler.atoms.name_of(atom).to_owned(),
        _ => panic!("the result is an error raise, not {result:?}"),
    }
}

/// Return the elements of the tuple value of `result`.
fn tuple_value(result: BifResult) -> Vec<Term> {
    match &result {
        BifResult::Value(Term::Tuple(elements)) => elements.to_vec(),
        _ => panic!("the result is a tuple, not {result:?}"),
    }
}

/// Return the atom name of the first element of the tuple value of
/// `result`.
fn tag(scheduler: &Scheduler, result: &BifResult) -> String {
    match result {
        BifResult::Value(Term::Tuple(elements)) => match &elements[0] {
            Term::Atom(atom) => scheduler.atoms.name_of(atom).to_owned(),
            other => panic!("the tag is an atom, not {other:?}"),
        },
        other => panic!("the result is a tuple, not {other:?}"),
    }
}

/// Return the bytes of the output list of a codec result.
fn output_bytes(result: &BifResult) -> Vec<u8> {
    let BifResult::Value(Term::Tuple(elements)) = result else {
        panic!("the result is a tuple, not {result:?}");
    };
    match &elements[1] {
        Term::Nil => Vec::new(),
        Term::Cons(_) => {
            let binaries: Vec<&Term> = elements[1].list_iter().unwrap().collect();
            assert_eq!(binaries.len(), 1);
            match binaries[0] {
                Term::Binary(bytes) => bytes.bit_bytes().to_vec(),
                other => panic!("the output is a binary, not {other:?}"),
            }
        }
        other => panic!("the output is a list, not {other:?}"),
    }
}

/// Queue `data` as one binary.
fn enqueue(scheduler: &mut Scheduler, stream: &Term, data: &[u8]) {
    let iovec = Term::list_from(vec![Term::binary(data.to_vec())]);
    let result = enqueue_nif(scheduler, stream, &iovec).unwrap();
    assert_eq!(atom_name(scheduler, result), "ok");
}

/// Return a compressed zlib stream of `data`.
fn compress(data: &[u8]) -> Vec<u8> {
    compress_with_window(data, 15)
}

/// Return a compressed zlib stream of `data` with the window bits
/// `window_bits`.
fn compress_with_window(data: &[u8], window_bits: i32) -> Vec<u8> {
    let mut deflate = Deflate::new_with_config(deflate_config(6, 8, window_bits, 8, 0).unwrap());
    let mut output = vec![0; data.len() * 2 + 64];
    assert_eq!(
        deflate
            .compress(data, &mut output, DeflateFlush::Finish)
            .unwrap(),
        Status::StreamEnd
    );
    output.truncate(deflate.total_out() as usize);
    output
}

/// Set the controlling process of `stream`.
fn set_controller(scheduler: &mut Scheduler, stream: &Term, controller: ProcessId) {
    let Term::Reference(reference) = stream else {
        panic!("the stream is a reference");
    };
    scheduler
        .resources
        .borrow_mut::<ZlibStreams>()
        .get_mut(*reference)
        .unwrap()
        .controller = controller;
}

#[test]
fn calls_each_binding_of_the_library_through_the_registry() {
    // The `nif_start` instruction finds each function of `nif_funcs` by its
    // name and arity (`zlib_nif.c:147-179`).
    use crate::runtime::interpreter::native::call;
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let integer = Term::Integer(1);
    let calls: [(&str, Vec<Term>); 14] = [
        ("open_nif", vec![]),
        ("close_nif", vec![stream.clone()]),
        (
            "deflateInit_nif",
            vec![
                stream.clone(),
                integer.clone(),
                integer.clone(),
                integer.clone(),
                integer.clone(),
                integer.clone(),
            ],
        ),
        ("deflateReset_nif", vec![stream.clone()]),
        ("deflateEnd_nif", vec![stream.clone()]),
        (
            "deflate_nif",
            vec![
                stream.clone(),
                integer.clone(),
                integer.clone(),
                integer.clone(),
            ],
        ),
        (
            "inflateInit_nif",
            vec![stream.clone(), integer.clone(), integer.clone()],
        ),
        ("inflateReset_nif", vec![stream.clone()]),
        ("inflateEnd_nif", vec![stream.clone()]),
        (
            "inflate_nif",
            vec![
                stream.clone(),
                integer.clone(),
                integer.clone(),
                integer.clone(),
            ],
        ),
        ("getStash_nif", vec![stream.clone()]),
        ("clearStash_nif", vec![stream.clone()]),
        ("setStash_nif", vec![stream.clone(), integer.clone()]),
        ("enqueue_nif", vec![stream.clone(), Term::nil()]),
    ];
    for (name, arguments) in calls {
        assert!(
            call(
                &mut scheduler,
                &crate::runtime::interpreter::native::tests::instance("zlib"),
                name,
                &arguments
            )
            .is_some(),
            "{name}"
        );
    }
}

#[test]
fn rejects_a_non_stream_for_each_nif() {
    let mut scheduler = scheduler();
    let nil = Term::nil();
    let integer = Term::Integer(1);
    let results = [
        close_nif(&mut scheduler, &nil).unwrap(),
        deflate_reset_nif(&mut scheduler, &nil).unwrap(),
        deflate_end_nif(&mut scheduler, &nil).unwrap(),
        deflate_nif(&mut scheduler, &nil, &integer, &integer, &integer).unwrap(),
        inflate_reset_nif(&mut scheduler, &nil).unwrap(),
        inflate_end_nif(&mut scheduler, &nil).unwrap(),
        inflate_nif(&mut scheduler, &nil, &integer, &integer, &integer).unwrap(),
        get_stash_nif(&mut scheduler, &nil).unwrap(),
        clear_stash_nif(&mut scheduler, &nil).unwrap(),
        set_stash_nif(&mut scheduler, &nil, &integer).unwrap(),
        enqueue_nif(&mut scheduler, &nil, &Term::nil()).unwrap(),
        deflate_init_nif(
            &mut scheduler,
            &nil,
            &integer,
            &integer,
            &integer,
            &integer,
            &integer,
        )
        .unwrap(),
        inflate_init_nif(&mut scheduler, &nil, &integer, &integer).unwrap(),
    ];
    for result in results {
        assert_eq!(raise_name(&scheduler, result), "badarg");
    }
}

#[test]
fn rejects_a_codec_size_that_is_not_a_c_int() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::binary(vec![]),
        &Term::Integer(1),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1),
        &Term::binary(vec![]),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
}

#[test]
fn gives_data_error_when_the_output_chunk_holds_no_byte() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    enqueue(&mut scheduler, &stream, b"abc");
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(0),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn accepts_each_inflate_flush_mode() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_inflate(&mut scheduler, &stream);
    for flush in [4, 5, 6] {
        let result = inflate_nif(
            &mut scheduler,
            &stream,
            &Term::Integer(1024),
            &Term::Integer(1024),
            &Term::Integer(flush),
        )
        .unwrap();
        assert_eq!(tag(&scheduler, &result), "finished");
    }
}

#[test]
fn maps_the_codec_failure_steps() {
    assert_eq!(
        codec_failure(Step::Raise("data_error")),
        Some(Native::Raise("data_error"))
    );
    assert_eq!(
        codec_failure(Step::Atom("mem_error")),
        Some(Native::Atom("mem_error"))
    );
    assert!(codec_failure(Step::Ok).is_none());
    assert!(codec_failure(Step::StreamEnd).is_none());
    assert!(codec_failure(Step::BufError).is_none());
    assert!(codec_failure(Step::NeedDict(1)).is_none());
}

#[test]
fn learns_the_window_size_of_an_automatic_header() {
    assert_eq!(learn_window_bits(32, &[0x78, 0x9c]), Some(15));
    assert_eq!(learn_window_bits(0, &[0x78, 0x9c]), Some(15));
    assert_eq!(learn_window_bits(32, &[0x18, 0x95]), Some(9));
    assert_eq!(learn_window_bits(32, &[0x1f, 0x8b]), Some(15));
    assert_eq!(learn_window_bits(16, &[0x1f, 0x8b]), Some(15));
    assert_eq!(learn_window_bits(16, &[0x78, 0x9c]), None);
    assert_eq!(learn_window_bits(0, &[0x1f, 0x8b]), None);
    assert_eq!(learn_window_bits(40, &[0x78, 0x9c]), None);
    assert_eq!(learn_window_bits(32, &[0x88, 0x1c]), Some(15));
    assert_eq!(learn_window_bits(32, &[0x18, 0x00]), None);
    assert_eq!(learn_window_bits(32, &[0x18]), None);
    assert_eq!(learn_window_bits(-15, &[0x78, 0x9c]), None);
}

#[test]
fn learns_the_window_size_of_a_header_split_across_calls() {
    // The C state reads the header across calls (`inflate.c:505-545`). The
    // first byte of the zlib header arrives in one `inflate_nif/4` call and
    // the rest in the next one. A reset must still keep the window size 9
    // of that header.
    let first = compress_with_window(b"hello hello hello", 9);
    let far: Vec<u8> = (0..600)
        .map(|value| (value % 251) as u8)
        .chain((0..600).map(|value| (value % 251) as u8))
        .collect();
    let second = compress_with_window(&far, 15);
    let (head, rest) = first.split_at(1);
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(32),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, head);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "finished");
    assert!(output_bytes(&result).is_empty());
    enqueue(&mut scheduler, &stream, rest);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"hello hello hello");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &second);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn resets_a_stream_after_an_invalid_window_size() {
    // `<<0x88, 0x1c>>` names a window of 16, which zlib stores in `wbits`
    // before it raises `data_error` (`inflate.c:539-545`). A reset gives
    // `ok`, and the next stream decodes.
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(32),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &[0x88, 0x1c]);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &compress(b"hello"));
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"hello");
}

#[test]
fn keeps_the_learned_window_size_across_inflate_reset() {
    // The C `inflateReset` keeps the window size that the first header told
    // (`inflate.c:136-171`, `539-545`). A window-9 stream teaches 9, and
    // the next stream can then use only that window. The repeated 600 bytes
    // need a distance of 600, so a window-9 inflate raises a data error.
    let first = compress_with_window(b"hello hello hello", 9);
    let far: Vec<u8> = (0..600)
        .map(|value| (value % 251) as u8)
        .chain((0..600).map(|value| (value % 251) as u8))
        .collect();
    let second = compress_with_window(&far, 15);
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(32),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &first);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"hello hello hello");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &second);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn keeps_the_learned_window_size_across_reset_with_a_zlib_wrapper() {
    // `windowBits` 0 takes the window size from the zlib header
    // (`inflate.c:539-545`), and `inflateReset` keeps it.
    let first = compress_with_window(b"hello hello hello", 9);
    let far: Vec<u8> = (0..600)
        .map(|value| (value % 251) as u8)
        .chain((0..600).map(|value| (value % 251) as u8))
        .collect();
    let second = compress_with_window(&far, 15);
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(0),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &first);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"hello hello hello");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &second);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn keeps_the_gzip_wrapper_across_inflate_reset() {
    // `windowBits` 16 takes the gzip header with no window size, and zlib
    // uses 15 (`inflate.c:513-515`). `inflateReset` keeps the gzip wrapper.
    let first = compress_with_window(b"hello", 25);
    let second = compress_with_window(b"world", 31);
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(16),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &first);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"hello");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &second);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"world");
}

#[test]
fn keeps_the_learned_window_size_across_the_automatic_reset() {
    // The automatic `reset` behavior calls `inflateReset` (`zlib_nif.c:937-958`),
    // which keeps the window size that the first header told.
    let first = compress_with_window(b"hello", 9);
    let far: Vec<u8> = (0..600)
        .map(|value| (value % 251) as u8)
        .chain((0..600).map(|value| (value % 251) as u8))
        .collect();
    let second = compress_with_window(&far, 15);
    let mut both = first;
    both.extend_from_slice(&second);
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(32),
        &Term::Integer(1),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &both);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"hello");
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn validates_the_deflate_parameters_of_the_pinned_zlib() {
    assert!(deflate_config(6, 8, 15, 8, 0).is_some());
    assert!(deflate_config(-1, 8, 15, 8, 0).is_some());
    assert!(deflate_config(6, 8, -15, 8, 0).is_some());
    assert!(deflate_config(6, 8, 31, 8, 0).is_some());
    assert!(deflate_config(6, 8, -8, 8, 0).is_none());
    assert!(deflate_config(6, 8, 8, 8, 0).is_some());
    assert!(deflate_config(6, 8, 24, 8, 0).is_none());
    assert!(deflate_config(6, 8, 25, 8, 0).is_some());
    assert!(deflate_config(10, 8, 15, 8, 0).is_none());
    assert!(deflate_config(6, 7, 15, 8, 0).is_none());
    assert!(deflate_config(6, 8, 15, 0, 0).is_none());
    assert!(deflate_config(6, 8, 15, 8, 5).is_none());
    assert!(deflate_config(6, 8, 100, 8, 0).is_none());
    assert!(deflate_config(6, 8, -16, 8, 0).is_none());
}

#[test]
fn validates_the_inflate_window_bits_of_the_pinned_zlib() {
    assert_eq!(inflate_window_bits(0), Some(0));
    assert_eq!(inflate_window_bits(15), Some(15));
    assert_eq!(inflate_window_bits(-15), Some(-15));
    assert_eq!(inflate_window_bits(31), Some(31));
    assert_eq!(inflate_window_bits(47), Some(47));
    assert_eq!(inflate_window_bits(-7), None);
    assert_eq!(inflate_window_bits(-16), None);
    assert_eq!(inflate_window_bits(17), None);
    assert_eq!(inflate_window_bits(23), None);
    assert_eq!(inflate_window_bits(48), None);
}

#[test]
fn reads_a_c_int_and_rejects_each_other_integer() {
    assert_eq!(integer(&Term::Integer(2)), Some(2));
    assert_eq!(integer(&Term::Integer(i64::from(i32::MAX) + 1)), None);
    assert_eq!(integer(&Term::binary(vec![])), None);
}

#[test]
fn rejects_a_term_that_is_not_a_stream_resource() {
    let mut scheduler = scheduler();
    let result = close_nif(&mut scheduler, &Term::nil()).unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
    let unknown = Term::Reference(Reference(999));
    let result = close_nif(&mut scheduler, &unknown).unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
}

#[test]
fn checks_the_controlling_process_before_each_other_check() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    set_controller(&mut scheduler, &stream, ProcessId(999));
    let integer = Term::Integer(1);
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = get_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = clear_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = set_stash_nif(&mut scheduler, &stream, &integer).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = deflate_init_nif(
        &mut scheduler,
        &stream,
        &integer,
        &integer,
        &integer,
        &integer,
        &integer,
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = inflate_init_nif(&mut scheduler, &stream, &integer, &integer).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = deflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = deflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = deflate_nif(&mut scheduler, &stream, &integer, &integer, &integer).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = inflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let result = inflate_nif(&mut scheduler, &stream, &integer, &integer, &integer).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
    let iovec = Term::list_from(vec![Term::binary(b"ab".to_vec())]);
    let result = enqueue_nif(&mut scheduler, &stream, &iovec).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_on_controlling_process");
}

#[test]
fn checks_the_stream_state_in_the_order_of_the_nif() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let integer = Term::Integer(1);
    let iovec = Term::list_from(vec![Term::binary(b"ab".to_vec())]);
    for result in [
        deflate_reset_nif(&mut scheduler, &stream).unwrap(),
        deflate_end_nif(&mut scheduler, &stream).unwrap(),
        deflate_nif(&mut scheduler, &stream, &integer, &integer, &integer).unwrap(),
        inflate_reset_nif(&mut scheduler, &stream).unwrap(),
        inflate_end_nif(&mut scheduler, &stream).unwrap(),
        inflate_nif(&mut scheduler, &stream, &integer, &integer, &integer).unwrap(),
        enqueue_nif(&mut scheduler, &stream, &iovec).unwrap(),
    ] {
        assert_eq!(raise_name(&scheduler, result), "not_initialized");
    }
    start_deflate(&mut scheduler, &stream);
    let result = deflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(6),
        &Term::Integer(8),
        &Term::Integer(15),
        &Term::Integer(8),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "already_initialized");
    let result = inflate_init_nif(&mut scheduler, &stream, &integer, &integer).unwrap();
    assert_eq!(raise_name(&scheduler, result), "already_initialized");
}

#[test]
fn starts_a_stream_with_each_invalid_parameter_as_stream_error() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = deflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(6),
        &Term::Integer(8),
        &Term::Integer(17),
        &Term::Integer(8),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "stream_error");
    let result = deflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::binary(vec![]),
        &Term::Integer(8),
        &Term::Integer(15),
        &Term::Integer(8),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(17),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "stream_error");
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::binary(vec![]),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
    // A closed stream is not in the `none` state of `open/0`.
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(15),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "already_initialized");
}

#[test]
fn runs_a_deflate_stream_and_checks_its_end() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    enqueue(&mut scheduler, &stream, b"hello hello hello world");
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(4),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "finished");
    let compressed = output_bytes(&result);
    assert_eq!(compressed, compress(b"hello hello hello world"));
    // A finished stream ends with `ok`.
    let result = deflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_initialized");
}

#[test]
fn gives_data_error_for_a_deflate_end_of_an_unfinished_stream() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "finished");
    let result = deflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn gives_data_error_for_a_deflate_end_with_queued_input() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    enqueue(&mut scheduler, &stream, b"abc");
    let result = deflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn resets_a_deflate_stream_and_drops_the_queue_and_stash() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    enqueue(&mut scheduler, &stream, b"abc");
    let result = set_stash_nif(&mut scheduler, &stream, &Term::Integer(1)).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = deflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = get_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "empty");
    enqueue(&mut scheduler, &stream, b"hello hello hello world");
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(4),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "finished");
}

#[test]
fn rejects_an_unknown_flush_mode_before_it_starts_the_stream() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(6),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "stream_error");
    // The stream did not start, so its end is `ok`.
    let result = deflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
}

#[test]
fn gives_continue_when_the_output_chunk_fills() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    let data: Vec<u8> = (0..100_000).map(|value| (value * 37 % 251) as u8).collect();
    enqueue(&mut scheduler, &stream, &data);
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(8192),
        &Term::Integer(8),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "continue");
}

#[test]
fn gives_mem_error_for_a_negative_output_chunk_size() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    enqueue(&mut scheduler, &stream, b"abc");
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(-1),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "mem_error");
}

#[test]
fn runs_an_inflate_stream_to_its_end() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_inflate(&mut scheduler, &stream);
    let compressed = compress(b"hello hello hello world");
    enqueue(&mut scheduler, &stream, &compressed);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "finished");
    assert_eq!(output_bytes(&result), b"hello hello hello world");
    let result = inflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
}

#[test]
fn gives_data_error_for_an_inflate_end_without_a_stream_end() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_inflate(&mut scheduler, &stream);
    enqueue(&mut scheduler, &stream, &compress(b"abc")[..4]);
    let _ = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    let result = inflate_end_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn resets_an_inflate_stream_with_its_window_bits() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(-15),
        &Term::Integer(2),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let mut deflate = Deflate::new_with_config(deflate_config(6, 8, -15, 8, 0).unwrap());
    let mut compressed = vec![0; 128];
    assert_eq!(
        deflate
            .compress(b"raw", &mut compressed, DeflateFlush::Finish)
            .unwrap(),
        Status::StreamEnd
    );
    compressed.truncate(deflate.total_out() as usize);
    enqueue(&mut scheduler, &stream, &compressed);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"raw");
    let result = inflate_reset_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &compressed);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"raw");
}

#[test]
fn gives_need_dictionary_for_a_stream_with_a_dictionary() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_inflate(&mut scheduler, &stream);
    let mut deflate = Deflate::new_with_config(deflate_config(6, 8, 15, 8, 0).unwrap());
    deflate
        .set_dictionary(b"the dictionary of the stream")
        .unwrap();
    let mut compressed = vec![0; 256];
    let status = deflate
        .compress(
            b"the dictionary of the stream again and again",
            &mut compressed,
            DeflateFlush::Finish,
        )
        .unwrap();
    assert_eq!(status, Status::StreamEnd);
    compressed.truncate(deflate.total_out() as usize);
    enqueue(&mut scheduler, &stream, &compressed);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "need_dictionary");
    let elements = tuple_value(result);
    assert!(matches!(elements[1], Term::Integer(_)));
}

#[test]
fn gives_each_end_of_stream_behavior() {
    let first = compress(b"first");
    let second = compress(b"second");
    let mut both = first.clone();
    both.extend_from_slice(&second);
    // `reset` continues with the second stream.
    let mut first_scheduler = scheduler();
    let first_stream = stream(&mut first_scheduler);
    let result = inflate_init_nif(
        &mut first_scheduler,
        &first_stream,
        &Term::Integer(15),
        &Term::Integer(1),
    )
    .unwrap();
    assert_eq!(atom_name(&first_scheduler, result), "ok");
    enqueue(&mut first_scheduler, &first_stream, &both);
    let result = inflate_nif(
        &mut first_scheduler,
        &first_stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&first_scheduler, &result), "continue");
    assert_eq!(output_bytes(&result), b"first");
    // The Erlang layer calls the NIF again with no input, and the reset
    // continues with the second stream.
    let result = inflate_nif(
        &mut first_scheduler,
        &first_stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&first_scheduler, &result), "finished");
    assert_eq!(output_bytes(&result), b"second");
    let result = inflate_end_nif(&mut first_scheduler, &first_stream).unwrap();
    assert_eq!(atom_name(&first_scheduler, result), "ok");
    // `cut` drops the second stream and keeps the end state.
    let mut second_scheduler = scheduler();
    let second_stream = stream(&mut second_scheduler);
    start_inflate(&mut second_scheduler, &second_stream);
    enqueue(&mut second_scheduler, &second_stream, &both);
    let result = inflate_nif(
        &mut second_scheduler,
        &second_stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(output_bytes(&result), b"first");
    let result = inflate_nif(
        &mut second_scheduler,
        &second_stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&second_scheduler, &result), "finished");
    assert!(output_bytes(&result).is_empty());
    let result = inflate_end_nif(&mut second_scheduler, &second_stream).unwrap();
    assert_eq!(atom_name(&second_scheduler, result), "ok");
    // `error` raises a data error.
    let mut third_scheduler = scheduler();
    let third_stream = stream(&mut third_scheduler);
    let result = inflate_init_nif(
        &mut third_scheduler,
        &third_stream,
        &Term::Integer(15),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(atom_name(&third_scheduler, result), "ok");
    enqueue(&mut third_scheduler, &third_stream, &both);
    let result = inflate_nif(
        &mut third_scheduler,
        &third_stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&third_scheduler, &result), "continue");
    assert_eq!(output_bytes(&result), b"first");
    // The Erlang layer calls the NIF again with no input, and the `error`
    // behavior raises the data error.
    let result = inflate_nif(
        &mut third_scheduler,
        &third_stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&third_scheduler, result), "data_error");
}

#[test]
fn keeps_an_unknown_end_of_stream_behavior_with_no_action() {
    // `zlib.erl` gives only 0, 1, and 2 to the NIF (`arg_eos_behavior/1`),
    // and the C `switch` has no default arm (`zlib_nif.c:937-958`). A
    // stream with another behavior keeps running the codec. The codec of
    // the finished stream gives `finished` with no output.
    let first = compress(b"first");
    let second = compress(b"second");
    let mut both = first;
    both.extend_from_slice(&second);
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = inflate_init_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(15),
        &Term::Integer(9),
    )
    .unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    enqueue(&mut scheduler, &stream, &both);
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(tag(&scheduler, &result), "continue");
    assert_eq!(output_bytes(&result), b"first");
    // The next call makes no progress on the queued bytes, so the codec
    // raises a data error (`zlib_nif.c:429-435`).
    let result = inflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1024),
        &Term::Integer(1024),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "data_error");
}

#[test]
fn runs_each_stash_nif() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = get_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "empty");
    let result = clear_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(raise_name(&scheduler, result), "error");
    let result = set_stash_nif(&mut scheduler, &stream, &Term::Integer(7)).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = set_stash_nif(&mut scheduler, &stream, &Term::Integer(8)).unwrap();
    assert_eq!(raise_name(&scheduler, result), "error");
    let result = get_stash_nif(&mut scheduler, &stream).unwrap();
    let elements = tuple_value(result);
    let [tag, value] = elements.as_slice() else {
        panic!("getStash gives a pair");
    };
    assert_eq!(scheduler.atoms.name_of(&taken_atom(tag)), "ok");
    assert!(matches!(value, Term::Integer(7)));
    let result = clear_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = get_stash_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "empty");
}

/// Return a copy of the atom of `term`.
fn taken_atom(term: &Term) -> Atom {
    match term {
        Term::Atom(atom) => atom.clone(),
        other => panic!("the term is an atom, not {other:?}"),
    }
}

#[test]
fn queues_the_iovec_of_enqueue_nif() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    let result = enqueue_nif(&mut scheduler, &stream, &Term::nil()).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    // An empty binary is no queue entry.
    let iovec = Term::list_from(vec![Term::binary(vec![]), Term::binary(b"ab".to_vec())]);
    let result = enqueue_nif(&mut scheduler, &stream, &iovec).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    // An element that is not a binary gives `badarg` and queues nothing.
    let iovec = Term::list_from(vec![Term::Integer(1)]);
    let result = enqueue_nif(&mut scheduler, &stream, &iovec).unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
    // An improper list gives `badarg`.
    let iovec = Term::list_with_tail(vec![Term::binary(b"ab".to_vec())], Term::Integer(1));
    let result = enqueue_nif(&mut scheduler, &stream, &iovec).unwrap();
    assert_eq!(raise_name(&scheduler, result), "badarg");
    // 257 elements leave a tail of one element.
    let elements: Vec<Term> = (0..257).map(|_| Term::binary(vec![b'a'])).collect();
    let iovec = Term::list_from(elements);
    let result = enqueue_nif(&mut scheduler, &stream, &iovec).unwrap();
    assert_eq!(tag(&scheduler, &result), "continue");
    let tail = tuple_value(result).remove(1);
    assert_eq!(tail.list_iter().unwrap().count(), 1);
}

#[test]
fn builds_each_result_shape() {
    let mut scheduler = scheduler();
    let cases = [
        Native::Atom("ok"),
        Native::Raise("data_error"),
        Native::Finished(vec![]),
        Native::Finished(b"ab".to_vec()),
        Native::Continue(b"ab".to_vec()),
        Native::NeedDictionary(7, b"ab".to_vec()),
        Native::ContinueInput(Term::Integer(1)),
        Native::Stash(Term::Integer(1)),
    ];
    for case in cases {
        let result = result(&mut scheduler, case).unwrap();
        assert!(matches!(
            result,
            BifResult::Value(_) | BifResult::Raise { .. }
        ));
    }
}

#[test]
fn maps_each_codec_error_to_the_erts_step() {
    assert_eq!(status_step::<DeflateError>(Ok(Status::Ok)), Step::Ok);
    assert_eq!(
        status_step::<DeflateError>(Ok(Status::StreamEnd)),
        Step::StreamEnd
    );
    assert_eq!(
        status_step::<DeflateError>(Ok(Status::BufError)),
        Step::BufError
    );
    assert_eq!(
        Step::from(DeflateError::StreamError),
        Step::Raise("stream_error")
    );
    assert_eq!(
        Step::from(DeflateError::DataError),
        Step::Raise("data_error")
    );
    assert_eq!(Step::from(DeflateError::MemError), Step::Atom("mem_error"));
    assert_eq!(
        Step::from(InflateError::NeedDict { dict_id: 7 }),
        Step::NeedDict(7)
    );
    assert_eq!(
        Step::from(InflateError::StreamError),
        Step::Raise("stream_error")
    );
    assert_eq!(
        Step::from(InflateError::DataError),
        Step::Raise("data_error")
    );
    assert_eq!(Step::from(InflateError::MemError), Step::Atom("mem_error"));
}

#[test]
fn removes_the_consumed_bytes_of_the_queue() {
    let mut queue = VecDeque::from([Binary::from(b"abc".to_vec()), Binary::from(b"de".to_vec())]);
    deq(&mut queue, 4);
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].bit_bytes(), b"e");
    deq(&mut queue, 1);
    assert!(queue.is_empty());
    let mut queue = VecDeque::from([Binary::from(b"abc".to_vec())]);
    deq(&mut queue, 0);
    assert_eq!(queue[0].bit_bytes(), b"abc");
}

#[test]
fn closes_a_stream_before_its_end() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    start_deflate(&mut scheduler, &stream);
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = deflate_nif(
        &mut scheduler,
        &stream,
        &Term::Integer(1),
        &Term::Integer(1),
        &Term::Integer(0),
    )
    .unwrap();
    assert_eq!(raise_name(&scheduler, result), "not_initialized");
}

#[test]
fn covers_the_codec_of_a_stream_with_no_operation() {
    let mut stream = ZStream::new(ProcessId(0));
    let native = codec(&mut stream, 1024, 1024, 0);
    assert_eq!(
        match native {
            Native::Raise(name) => name,
            _ => panic!("a stream with no state raises"),
        },
        "stream_error"
    );
    stream.state = State::Closed;
    let native = codec(&mut stream, 1024, 1024, 0);
    assert!(matches!(native, Native::Raise("stream_error")));
}

#[test]
fn opens_and_closes_a_stream() {
    let mut scheduler = scheduler();
    let stream = stream(&mut scheduler);
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert_eq!(atom_name(&scheduler, result), "ok");
    let result = close_nif(&mut scheduler, &stream).unwrap();
    assert!(matches!(result, BifResult::Raise { .. }));
}
