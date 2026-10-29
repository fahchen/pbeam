use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

const PROBE: &[u8] = include_bytes!("../../fixtures/zlib/zlib_probe.beam");

/// Load `zlib_probe` with the pinned `zlib` and `lists` modules. The pinned
/// `zlib` is the preloaded module of OTP 29.1.1, and its `on_load/0` binds
/// the static NIF library.
fn program() -> BeamProgram {
    let zlib = common::pinned_beam("zlib");
    let lists = common::pinned_beam("lists");
    BeamProgram::load(&[&zlib, &lists, PROBE]).unwrap()
}

/// Run `zlib_probe:Function()` on PBeam and return its value.
fn run(program: &BeamProgram, function: &str) -> Term {
    program.execute("zlib_probe", function, &[]).unwrap().term
}

/// Run `zlib_probe:Function(Arguments)` on PBeam and return its value.
fn call(program: &BeamProgram, function: &str, arguments: &[Term]) -> Term {
    let arguments: Vec<_> = arguments
        .iter()
        .cloned()
        .map(|term| program.host_term(term))
        .collect();
    program
        .execute("zlib_probe", function, &arguments)
        .unwrap()
        .term
}

/// Evaluate the expression on the pinned ERTS with the fixture on the code
/// path, and return the standard output.
fn reference(expression: &str) -> String {
    let output = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/zlib",
            "-eval",
            &format!("{expression}, halt()."),
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// Return whether the pinned ERTS decompresses `hex` to `expected`, an
/// Erlang expression over the fixture.
fn erts_decodes(hex: &str, expected: &str) -> bool {
    reference(&format!(
        "io:format(\"~p~n\",[zlib_probe:uncompress_hex(<<\"{hex}\">>) =:= {expected}])"
    ))
    .trim()
        == "true"
}

/// Return the elements of a tuple.
fn elements(term: &Term) -> &[Term] {
    match term {
        Term::Tuple(elements) => elements,
        other => panic!("the term is a tuple, not {other:?}"),
    }
}

/// Return the elements of a proper list.
fn list(term: &Term) -> Vec<&Term> {
    term.list_iter()
        .unwrap_or_else(|| panic!("the term is a list, not {term:?}"))
        .collect()
}

/// Return the text of a binary of ASCII characters.
fn binary_text(term: &Term) -> String {
    match term {
        Term::Binary(bytes) => String::from_utf8(
            bytes
                .bytes()
                .expect("the term is a binary of whole bytes")
                .to_vec(),
        )
        .unwrap(),
        other => panic!("the term is a binary, not {other:?}"),
    }
}

/// Return the `~w` text of an atom, an integer, a list of those, or a tuple
/// of those.
fn plain_text(program: &BeamProgram, term: &Term) -> String {
    match term {
        Term::Atom(atom) => program.atoms().name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::Binary(bytes) => format!(
            "<<\"{}\">>",
            String::from_utf8_lossy(bytes.bytes().expect("the term is a binary of whole bytes"))
        ),
        Term::Nil => "[]".to_owned(),
        Term::Cons(_) => {
            let elements: Vec<String> = list(term)
                .iter()
                .map(|element| plain_text(program, element))
                .collect();
            format!("[{}]", elements.join(","))
        }
        Term::Tuple(elements) => {
            let elements: Vec<String> = elements
                .iter()
                .map(|element| plain_text(program, element))
                .collect();
            format!("{{{}}}", elements.join(","))
        }
        other => panic!("the term has no `~w` text here: {other:?}"),
    }
}

/// Return the bytes of `data/0` of the fixture.
fn data() -> Vec<u8> {
    "The quick brown fox jumps over the lazy dog. "
        .repeat(1000)
        .into_bytes()
}

/// Return the bytes of `small/0` of the fixture.
fn small() -> Term {
    Term::binary(b"hello hello hello world".to_vec())
}

/// Return the bytes of two copies of `small/0` of the fixture.
fn double_small() -> Term {
    Term::binary(b"hello hello hello worldhello hello hello world".to_vec())
}

#[test]
fn matches_the_pinned_erts_deflate_bytes() {
    // The deflate bytes of each level 0 to 9, each strategy, and each window
    // of `small/0` (zlib headers, the raw stream, and the gzip header).
    // `zlib-rs` gives the pinned zlib 1.3.2 bytes for these cases.
    let program = program();
    let expected =
        reference("[io:format(\"~s ~s~n\",[Label,Hex]) || {Label,Hex} <- zlib_probe:exact()]");
    let expected: Vec<(String, String)> = expected
        .lines()
        .map(|line| {
            let (label, hex) = line.rsplit_once(' ').unwrap();
            (label.to_owned(), hex.to_owned())
        })
        .collect();
    let actual: Vec<(String, String)> = list(&run(&program, "exact"))
        .iter()
        .map(|entry| {
            let [label, hex] = elements(entry) else {
                panic!("each entry is a pair");
            };
            (binary_text(label), binary_text(hex))
        })
        .collect();
    assert_eq!(actual.len(), expected.len());
    for ((label, actual), (expected_label, expected)) in actual.iter().zip(&expected) {
        assert_eq!(label, expected_label);
        if label == "level 1" {
            // `zlib-rs` gives different level 1 bytes for this input. Each
            // stream holds the same decompressed bytes.
            assert_ne!(actual, expected);
            for hex in [actual, expected] {
                assert_eq!(
                    call(
                        &program,
                        "uncompress_hex",
                        &[Term::binary(hex.as_bytes().to_vec())]
                    ),
                    small()
                );
            }
            assert!(erts_decodes(actual, "zlib_probe:small()"));
        } else {
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn records_the_compressed_byte_difference_of_the_filtered_strategy() {
    // The pinned zlib 1.3.2 and `zlib-rs` give different compressed bytes
    // for level 6 and the `filtered` strategy. The decompressed bytes are
    // the same, and each library can decompress the output of the other.
    // `docs/erts-differences.md` records this difference.
    let program = program();
    let reference_hex = reference("io:format(\"~s~n\",[zlib_probe:filtered_large()])");
    let reference_hex = reference_hex.trim().to_owned();
    let actual_hex = binary_text(&run(&program, "filtered_large"));
    assert_ne!(actual_hex, reference_hex);
    let data = Term::binary(data());
    assert_eq!(
        call(
            &program,
            "uncompress_hex",
            &[Term::binary(reference_hex.into_bytes())]
        ),
        data
    );
    assert_eq!(
        call(
            &program,
            "uncompress_hex",
            &[Term::binary(actual_hex.clone().into_bytes())]
        ),
        data
    );
    assert!(erts_decodes(&actual_hex, "zlib_probe:data()"));
}

#[test]
fn roundtrips_the_one_shot_functions_as_pinned_erts() {
    let program = program();
    assert_eq!(
        plain_text(&program, &run(&program, "roundtrips")),
        reference("[io:format(\"~w~n\",[zlib_probe:roundtrips()])]").trim()
    );
}

#[test]
fn matches_the_pinned_erts_flush_bytes() {
    // The flush sequence has different bytes in `zlib-rs`. The stream after
    // `deflateReset/1` has the pinned bytes. Each stream holds the same
    // decompressed bytes.
    let program = program();
    let expected = reference("[io:format(\"~s~n\",[Hex]) || Hex <- zlib_probe:flushes()]");
    let expected: Vec<&str> = expected.lines().collect();
    let actual: Vec<String> = list(&run(&program, "flushes"))
        .iter()
        .map(|hex| binary_text(hex))
        .collect();
    assert_eq!(actual.len(), expected.len());
    assert_ne!(actual[0], expected[0]);
    assert_eq!(actual[1], expected[1]);
    for hex in [&actual[0], expected[0]] {
        assert_eq!(
            call(
                &program,
                "uncompress_hex",
                &[Term::binary(hex.as_bytes().to_vec())]
            ),
            double_small()
        );
    }
    assert!(erts_decodes(
        &actual[0],
        "<<(zlib_probe:small())/binary, (zlib_probe:small())/binary>>"
    ));
}

#[test]
fn matches_the_pinned_erts_safe_inflate_chunks() {
    let program = program();
    let actual: Vec<String> = list(&run(&program, "safe_chunks"))
        .iter()
        .map(|chunk| {
            let [tag, size] = elements(chunk) else {
                panic!("each chunk is a pair");
            };
            format!("{} {}", binary_text(tag), plain_text(&program, size))
        })
        .collect();
    let expected =
        reference("[io:format(\"~s ~p~n\",[Tag,Size]) || {Tag,Size} <- zlib_probe:safe_chunks()]");
    assert_eq!(format!("{}\n", actual.join("\n")), expected);
}

#[test]
fn matches_the_pinned_erts_end_of_stream_behaviors() {
    let program = program();
    let eos = run(&program, "eos");
    let actual = elements(&eos);
    assert_eq!(actual.len(), 3);
    let actual = format!(
        "{} {} {}",
        binary_text(&actual[0]),
        binary_text(&actual[1]),
        binary_text(&actual[2])
    );
    let expected = reference("{A,B,C} = zlib_probe:eos(), io:format(\"~s ~s ~s~n\",[A,B,C])");
    assert_eq!(actual, expected.trim());
}

#[test]
fn matches_the_pinned_erts_errors() {
    let program = program();
    let actual: Vec<String> = list(&run(&program, "errors"))
        .iter()
        .map(|error| binary_text(error))
        .collect();
    let expected = reference("[io:format(\"~s~n\",[Error]) || Error <- zlib_probe:errors()]");
    assert_eq!(format!("{}\n", actual.join("\n")), expected);
}

#[test]
fn raises_not_on_controlling_process_for_another_process() {
    let program = program();
    let actual = binary_text(&run(&program, "controller"));
    let expected = reference("io:format(\"~s~n\",[zlib_probe:controller()])");
    assert_eq!(actual, expected.trim());
}

#[test]
fn gives_the_stream_to_a_new_controlling_process() {
    let program = program();
    let actual = binary_text(&run(&program, "set_controller"));
    let expected = reference("io:format(\"~s~n\",[zlib_probe:set_controller()])");
    assert_eq!(actual.trim(), expected.trim());
    assert_eq!(actual.trim(), "badarg badarg ok not_on_controlling_process");
}

#[test]
fn stops_the_run_at_each_unbound_nif() {
    // The five functions that this task leaves without a binding have the
    // runtime error of the static library.
    let program = program();
    for (function, expected) in [
        (
            "unsupported_deflate_dictionary",
            "zlib:deflateSetDictionary_nif/2 NIF is not supported",
        ),
        (
            "unsupported_deflate_params",
            "zlib:deflateParams_nif/3 NIF is not supported",
        ),
        (
            "unsupported_inflate_dictionary",
            "zlib:inflateSetDictionary_nif/2 NIF is not supported",
        ),
        (
            "unsupported_inflate_get_dictionary",
            "zlib:inflateGetDictionary_nif/1 NIF is not supported",
        ),
    ] {
        let error = program
            .execute("zlib_probe", function, &[])
            .unwrap_err()
            .to_string();
        assert_eq!(error, expected);
    }
}
