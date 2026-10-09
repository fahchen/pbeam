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
