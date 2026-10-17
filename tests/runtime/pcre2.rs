use super::*;

#[test]
fn decodes_the_serialized_bytes_of_a_compiled_code() {
    // `pcre2_serialize_decode` rebuilds the code with the default allocator,
    // as `the_precomp_general_ctx` does for `re:import/1`
    // (erl_bif_re.c:761-763).
    let code = compile(b"(a)", 0, None, None).unwrap();
    let encoded = code.serialize();
    let Deserialized::Code(code) = deserialize(&encoded) else {
        panic!("the serialized bytes of a code decode");
    };
    assert_eq!(code.capture_count(), 1);
    assert_eq!(code.newline(), NEWLINE_LF);
}

#[test]
fn reports_incompatible_and_invalid_serialized_bytes() {
    // A wrong magic number, PCRE2 version, or PCRE2 configuration makes
    // `re:import/1` re-compile the source (erl_bif_re.c:783-788). Each
    // other error gives `badarg` (erl_bif_re.c:790-795). The pbeam check
    // also stops bytes that PCRE2 would read past, because ERTS gives it no
    // length.
    let code = compile(b"a", 0, None, None).unwrap();
    let encoded = code.serialize();
    let word = std::mem::size_of::<usize>();
    let (header, tables, blocksize_offset, count_offset, magic_offset, code_start_offset) =
        // SAFETY: the layout functions read no memory.
        unsafe {
            (
                pbeam_pcre2_serialized_header_size(),
                pbeam_pcre2_serialized_tables_size(),
                pbeam_pcre2_serialized_blocksize_offset(),
                pbeam_pcre2_serialized_number_of_codes_offset(),
                pbeam_pcre2_serialized_magic_offset(),
                pbeam_pcre2_serialized_code_start_offset(),
            )
        };
    let mut bad_magic = encoded.clone();
    bad_magic[..4].fill(0);
    assert!(matches!(
        deserialize(&bad_magic),
        Deserialized::Incompatible
    ));
    let mut bad_version = encoded.clone();
    bad_version[4..8].fill(0);
    assert!(matches!(
        deserialize(&bad_version),
        Deserialized::Incompatible
    ));
    let mut bad_config = encoded.clone();
    bad_config[8..12].fill(0);
    assert!(matches!(
        deserialize(&bad_config),
        Deserialized::Incompatible
    ));
    let mut no_codes = encoded.clone();
    no_codes[count_offset..count_offset + 4].fill(0);
    assert!(matches!(deserialize(&no_codes), Deserialized::Invalid));
    assert!(matches!(
        deserialize(&encoded[..header - 1]),
        Deserialized::Invalid
    ));
    assert!(matches!(
        deserialize(&encoded[..header + tables]),
        Deserialized::Invalid
    ));
    let mut forged = encoded.clone();
    let at = header + tables + blocksize_offset;
    forged[at..at + word].copy_from_slice(&usize::MAX.to_ne_bytes());
    assert!(matches!(deserialize(&forged), Deserialized::Invalid));
    // The magic number of the first code block, after a valid header and
    // block size, makes the decode itself fail.
    let mut bad_code = encoded.clone();
    let at = header + tables + magic_offset;
    bad_code[at..at + 4].fill(0);
    assert!(matches!(deserialize(&bad_code), Deserialized::Invalid));
    // A `code_start` outside the block makes the matcher read outside it
    // (`pcre2_match.c:8018`).
    let mut bad_start = encoded.clone();
    let at = header + tables + code_start_offset;
    bad_start[at..at + word].copy_from_slice(&usize::MAX.to_ne_bytes());
    assert!(matches!(deserialize(&bad_start), Deserialized::Invalid));
}

#[test]
fn gives_unknown_error_for_a_code_that_pcre2_does_not_know() {
    // ERTS: "Unknown error (%d)" for PCRE2_ERROR_BADDATA
    // (erl_bif_re.c:812-815).
    assert_eq!(error_message(99999), b"Unknown error (99999)");
    assert_eq!(error_message(114), b"missing closing parenthesis");
}

#[test]
fn stops_at_the_stack_guard_on_a_small_stack() {
    // On a 64 KiB stack, 100 nested lookaheads reach the 10 KiB margin
    // of the guard (erl_bif_re.c:157-169). PCRE2 then returns error 133
    // at offset 0, and the thread does not crash.
    let pattern = [b"(?=".repeat(100), b")".repeat(100)].concat();
    let result = std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || compile(&pattern, 0, None, None).map(|code| code.capture_count()))
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(
        result,
        Err(CompileError {
            message: b"parentheses are too deeply nested (stack check)".to_vec(),
            offset: 0,
        })
    );
}

#[test]
fn reads_only_the_offset_pairs_that_the_match_wrote() {
    // The match data allocates the pairs of the precompiled pattern term,
    // but a match writes only the pairs of the code. A forged pair count
    // above the pairs of the code must not widen the borrow into
    // uninitialized memory, so the slice stops at the returned count.
    let code = compile(b"a", 0, None, None).unwrap();
    let mut data = MatchData::new(3);
    data.set_loops_left(i32::MAX);
    assert_eq!(execute(&code, &data, None, b"a", 0, 0), 1);
    assert_eq!(data.ovector(1), [0, 1]);
    assert_eq!(data.ovector(1).len(), 2);
}

#[test]
fn resumes_a_match_that_uses_its_loop_limit() {
    // A match that uses its loop limit returns `PCRE2_ERROR_LOOP_LIMIT` and
    // keeps the position of the match in the restart data of the match
    // data; a fresh loop limit and `resume` continue it, as the trap to
    // `erlang:re_run_trap/3` does (`erl_bif_re.c:1852-1866`).
    let code = compile(b"a*b", 0, None, None).unwrap();
    let subject = vec![b'a'; 200000];
    let mut data = MatchData::new(2);
    data.set_loops_left(1);
    assert_eq!(
        execute(&code, &data, None, &subject, 0, 0),
        ERROR_LOOP_LIMIT
    );
    data.set_loops_left(i32::MAX);
    data.clear_restart();
    assert_eq!(data.resume(), ERROR_NOMATCH);
}
