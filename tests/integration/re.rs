use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const RE_PROBE: &[u8] = include_bytes!("../../fixtures/re/re_probe.beam");

/// Load `re_probe` with the pinned `prim_tty` and `re` modules.
fn program() -> BeamProgram {
    BeamProgram::load(&[RE_PROBE, &pinned_beam("prim_tty"), &pinned_beam("re")]).unwrap()
}

/// Run `re_probe:Function()` under the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> String {
    reference_of(&format!("re_probe:{function}()"))
}

/// Evaluate `Expression` with `re_probe` under the pinned ERTS and return
/// its `~w` output.
fn reference_of(expression: &str) -> String {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/re",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout).unwrap()
}

/// Return `{Count, Unicode, Crlf}`, the `info/1` result of the fixture.
fn info(count: i64, unicode: i64, crlf: i64) -> Term {
    Term::tuple(vec![
        Term::Integer(count),
        Term::Integer(unicode),
        Term::Integer(crlf),
    ])
}

/// Return `{error, {Message, Offset}}` with `Message` as a list of
/// character codes, as `build_compile_error` (erl_bif_re.c:802-836) makes
/// it.
fn compile_error(program: &BeamProgram, message: &str, offset: i64) -> Term {
    let message = message.bytes().map(|byte| Term::Integer(byte.into()));
    Term::tuple(vec![
        Term::Atom(program.atom("error").unwrap()),
        Term::tuple(vec![Term::list_from(message), Term::Integer(offset)]),
    ])
}

/// Return the `~w` text of a compile error, which writes the message as its
/// list of character codes.
fn compile_error_text(message: &str, offset: i64) -> String {
    let codes: Vec<String> = message.bytes().map(|byte| byte.to_string()).collect();
    format!("{{error,{{[{}],{offset}}}}}", codes.join(","))
}

#[test]
fn compiles_the_ansi_regexp_of_prim_tty_as_pinned_erts() {
    // `prim_tty:init/1` compiles the ANSI regular expression with
    // `[unicode]` (prim_tty.erl:238). The result has no capture group, the
    // unicode flag, and the default newline `lf`. The fifth element is a
    // reference, and each compile gives a new one (erl_bif_re.c:600-609).
    let program = program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program.execute("re_probe", "main", &[]).unwrap().term,
        Term::list_from(vec![
            Term::Integer(0),
            Term::Integer(1),
            Term::Integer(0),
            atom("reference"),
            atom("false"),
        ])
    );
    assert_eq!(reference("main"), "[0,1,0,reference,false]\n");
}

#[test]
fn returns_the_compile_result_of_each_option_as_pinned_erts() {
    // The capture count, the unicode flag, and the newline flag
    // (build_compile_result, erl_bif_re.c:581-615). `UseCrlf` is 1 for the
    // newline `crlf`, `anycrlf`, and `any`, also from `(*CRLF)` and `(*ANY)`
    // in the pattern. `(*UTF)` does not set the unicode flag. The last
    // `newline` option applies.
    let program = program();
    assert_eq!(
        program.execute("re_probe", "options", &[]).unwrap().term,
        Term::list_from(vec![
            info(0, 0, 0),
            info(2, 0, 0),
            info(0, 1, 0),
            info(0, 0, 0),
            info(0, 0, 0),
            info(0, 0, 1),
            info(0, 0, 0),
            info(0, 0, 1),
            info(0, 0, 1),
            info(0, 0, 1),
            info(0, 0, 1),
            info(0, 0, 0),
            info(0, 0, 0),
            info(2, 0, 0),
            info(0, 0, 0),
            info(0, 0, 0),
        ])
    );
    assert_eq!(
        reference("options"),
        "[{0,0,0},{2,0,0},{0,1,0},{0,0,0},{0,0,0},{0,0,1},{0,0,0},{0,0,1},{0,0,1},{0,0,1},{0,0,1},{0,0,0},{0,0,0},{2,0,0},{0,0,0},{0,0,0}]\n"
    );
}

#[test]
fn returns_the_pcre2_message_and_offset_of_each_compile_error_as_pinned_erts() {
    // The PCRE2 10.47 message and offset (build_compile_error,
    // erl_bif_re.c:802-836): a missing parenthesis, a missing `]`, a bad
    // quantifier, a duplicate name, a byte that is not UTF-8, `never_utf`
    // with `unicode` or `(*UTF)`, and 10001 nested groups, one more than
    // `PARENS_NEST_LIMIT` (local_config.h).
    let program = program();
    let errors = [
        ("missing closing parenthesis", 5),
        ("missing closing parenthesis", 5),
        ("missing terminating ] for character class", 1),
        ("numbers out of order in {} quantifier", 5),
        (
            "two named subpatterns have the same name (PCRE2_DUPNAMES not set)",
            12,
        ),
        ("UTF-8 error: illegal byte (0xfe or 0xff)", 0),
        ("using UTF is disabled by the application", 0),
        ("using UTF is disabled by the application", 6),
        ("parentheses are too deeply nested", 30003),
    ];
    assert_eq!(
        program.execute("re_probe", "errors", &[]).unwrap().term,
        Term::list_from(
            errors
                .iter()
                .map(|&(message, offset)| compile_error(&program, message, offset))
        )
    );
    let texts: Vec<String> = errors
        .iter()
        .map(|&(message, offset)| compile_error_text(message, offset))
        .collect();
    assert_eq!(reference("errors"), format!("[{}]\n", texts.join(",")));
}

#[test]
fn raises_badarg_for_bad_options_and_patterns_as_pinned_erts() {
    // Options that are not a list, each option that `parse_options` rejects
    // (erl_bif_re.c:350-545), each execution option (erl_bif_re.c:914-916),
    // also with `unicode`, and each pattern that is not iodata
    // (erl_bif_re.c:927-929). A bad option raises before a bad pattern.
    let program = program();
    let badarg = Term::tuple(vec![
        Term::Atom(program.atom("error").unwrap()),
        Term::Atom(program.atom("badarg").unwrap()),
    ]);
    assert_eq!(
        program.execute("re_probe", "badarg", &[]).unwrap().term,
        Term::list_from(vec![badarg; 26])
    );
    assert_eq!(
        reference("badarg"),
        format!("[{}]\n", vec!["{error,badarg}"; 26].join(","))
    );
}

#[test]
fn compiles_iodata_patterns_as_pinned_erts() {
    // Without `unicode`, a pattern is iodata: a byte list, nested lists of
    // bytes and binaries, an empty binary, and an empty list
    // (get_iolist_as_bytes, erl_bif_re.c:850-889).
    let program = program();
    assert_eq!(
        program.execute("re_probe", "iodata", &[]).unwrap().term,
        Term::list_from(vec![
            info(1, 0, 0),
            info(2, 0, 0),
            info(0, 0, 0),
            info(0, 0, 0)
        ])
    );
    assert_eq!(reference("iodata"), "[{1,0,0},{2,0,0},{0,0,0},{0,0,0}]\n");
}

#[test]
fn compiles_unicode_character_lists_through_re_ucompile_as_pinned_erts() {
    // With `unicode`, a pattern that is not a binary traps to
    // `re:ucompile/2` (erl_bif_re.c:921-923), which converts it to UTF-8
    // and compiles the binary. The offset of an error is a byte offset in
    // that binary.
    let program = program();
    assert_eq!(
        program
            .execute("re_probe", "unicode_list", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            info(1, 1, 0),
            info(2, 1, 0),
            compile_error(&program, "missing closing parenthesis", 1),
        ])
    );
    assert_eq!(
        reference("unicode_list"),
        format!(
            "[{{1,1,0}},{{2,1,0}},{}]\n",
            compile_error_text("missing closing parenthesis", 1)
        )
    );
}

#[test]
fn exports_compiled_patterns_as_pinned_erts() {
    // The `export` option gives `{ok, {re_exported_pattern, Header, Source,
    // Options, Encoded}}` (build_compile_export, erl_bif_re.c:644-700).
    // `Header` is "re-PCRE2", the zlib CRC-32 of `Encoded`, the encode
    // version 1, and the unicode byte (erl_bif_re.c:617-639). `Source` is
    // the pattern bytes, also of iodata and of a character list that
    // `re:ucompile/2` converts. `Options` is the option term as given.
    // `Encoded` is the `pcre2_serialize_encode` form, which depends on the
    // host, so each binary is compared with the ERTS bytes on this host.
    // `Encoded` holds the compile options and the bsr convention of the
    // code, so this comparison checks the PCRE2 bits of each compile
    // option, and that the last bsr option applies. A compile error is not
    // exported.
    let program = program();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let result = program.execute("re_probe", "export", &[]).unwrap();
    let Some(results) = result
        .term
        .proper_list()
        .map(|results| results.collect::<Vec<_>>())
    else {
        panic!("export/0 returns a list, got {result:?}");
    };
    let (failure, results) = results.split_last().unwrap();
    assert_eq!(
        *failure,
        &compile_error(&program, "missing closing parenthesis", 1)
    );
    let mut expected = vec![
        (b"a".as_slice(), vec![atom("export")], 0),
        (b"ab", vec![atom("export"), atom("caseless")], 0),
        (
            "a\u{100}".as_bytes(),
            vec![atom("unicode"), atom("export")],
            1,
        ),
    ];
    let single_options = [
        "anchored",
        "caseless",
        "dollar_endonly",
        "dotall",
        "dupnames",
        "extended",
        "firstline",
        "multiline",
        "ungreedy",
        "ucp",
        "no_auto_capture",
        "no_start_optimize",
        "bsr_anycrlf",
        "bsr_unicode",
        "never_utf",
    ];
    let option_lists = single_options.iter().map(|&option| vec![option]).chain([
        vec!["bsr_anycrlf", "bsr_unicode"],
        vec!["bsr_unicode", "bsr_anycrlf"],
    ]);
    for options in option_lists {
        let given = std::iter::once("export").chain(options).map(atom);
        expected.push((b"a", given.collect(), 0));
    }
    assert_eq!(results.len(), expected.len());
    let mut bytes = Vec::new();
    for (result, (source, options, unicode)) in results.iter().zip(expected) {
        let Some(elements) = result
            .proper_list()
            .map(|elements| elements.collect::<Vec<_>>())
        else {
            panic!("expected [Header, Source, Options, Encoded], got {result:?}");
        };
        let [
            Term::Binary(header),
            Term::Binary(pattern),
            given,
            Term::Binary(encoded),
        ] = elements.as_slice()
        else {
            panic!("expected three binaries and an option list, got {elements:?}");
        };
        assert_eq!(header.len(), 14);
        assert_eq!(&header[..8], b"re-PCRE2");
        assert_eq!(header[12..], [1, unicode]);
        assert_eq!(&pattern[..], source);
        assert_eq!(**given, Term::list_from(options));
        bytes.push([header, pattern, encoded]);
    }
    // Each compile option changes `Encoded`. `bsr_unicode` is the default,
    // and a later bsr option replaces an earlier one.
    let encoded: Vec<&[u8]> = bytes.iter().map(|[_, _, encoded]| &encoded[..]).collect();
    let plain = encoded[0];
    let (by_option, bsr_orders) = encoded[3..].split_at(single_options.len());
    for (option, &bytes) in single_options.iter().zip(by_option) {
        assert_eq!(bytes == plain, *option == "bsr_unicode", "{option}");
    }
    assert_eq!(bsr_orders[0], plain);
    let bsr_anycrlf = single_options
        .iter()
        .position(|&option| option == "bsr_anycrlf");
    assert_eq!(bsr_orders[1], by_option[bsr_anycrlf.unwrap()]);
    let text = |bytes: &[u8]| {
        let bytes: Vec<String> = bytes.iter().map(u8::to_string).collect();
        format!("[{}]", bytes.join(","))
    };
    let exports: Vec<String> = bytes
        .iter()
        .map(|binaries| {
            let binaries: Vec<String> = binaries.iter().map(|bytes| text(bytes)).collect();
            format!("[{}]", binaries.join(","))
        })
        .collect();
    assert_eq!(
        reference_of(
            "[[binary_to_list(B) || B <- [H, S, E]] \
             || [H, S, _, E] <- lists:droplast(re_probe:export())]"
        ),
        format!("[{}]\n", exports.join(","))
    );
    assert_eq!(
        reference_of("lists:last(re_probe:export())"),
        format!("{}\n", compile_error_text("missing closing parenthesis", 1))
    );
}

#[test]
fn raises_badarg_for_a_unicode_pattern_that_is_not_chardata() {
    // ERTS raises `badarg` through `re:ucompile/2` for a `unicode` pattern
    // that is not chardata: it catches the `badarg` of
    // `unicode:characters_to_binary/2`, takes the first frame of
    // `erlang:error(new_stacktrace, [RE, Options])`, and raises `badarg`
    // again with `erlang:raise/3` (`re.erl:1742-1751`). PBeam gives
    // `erlang:error/2` this first frame, so the match succeeds.
    // `erlang:raise/3` keeps the frame `{re, compile, [RE, Options], []}`
    // (`bif.c:1626-1687`). The raise then also gives the frame of the caller
    // of the compile, which PBeam takes from the frames of the process. ERTS
    // also gives the location of each frame; PBeam gives no location (row
    // "Exception stack trace" in `docs/erts-differences.md`).
    let program = program();
    let error = program
        .execute(
            "re_probe",
            "unsupported",
            &[program.host_term(Term::Atom(program.atom("chardata").unwrap()))],
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "root process exited with {badarg,[{re,compile,[foo,[unicode]],[]},\
         {re_probe,unsupported,1,[]}]}"
    );
    assert_eq!(
        reference_of("try re_probe:unsupported(chardata) catch error:R -> R end"),
        "badarg\n"
    );
}
