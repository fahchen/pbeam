use super::*;

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
