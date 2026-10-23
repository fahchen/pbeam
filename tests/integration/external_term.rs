#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/external_term/external_term_probe.beam");

/// Return the fixture program with the pinned `erlang` module.
fn load() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[&erlang, PROBE]).unwrap()
}

/// Return the atom `name` of the program.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

/// Run `function/0` of the fixture.
fn run(program: &BeamProgram, function: &str) -> Term {
    program
        .execute("external_term_probe", function, &[])
        .unwrap()
        .term
}

/// Return `bytes` as a binary term.
fn binary(bytes: &[u8]) -> Term {
    Term::binary(bytes.to_vec())
}

#[test]
fn encodes_and_decodes_bitstrings_as_pinned_erts() {
    // `external_term_probe:bits/0` on the pinned ERTS, measured with
    // `mise run erts-eval --pa fixtures/external_term`. `enc_term` writes
    // `BIT_BINARY_EXT` with the bit count of the last byte and clears its
    // unused bits (`external.c:3861`, `3916-3935`); `dec_term` raises
    // `badarg` for a bit count outside 1 to 8 or no bytes
    // (`external.c:4895-4923`).
    let program = load();
    let nested = [
        131, 104, 3, 77, 0, 0, 0, 1, 1, 128, 108, 0, 0, 0, 1, 77, 0, 0, 0, 1, 2, 128, 106, 116, 0,
        0, 0, 1, 119, 1, 97, 77, 0, 0, 0, 1, 7, 6,
    ];
    let encoded = Term::list_from(vec![
        binary(&[131, 77, 0, 0, 0, 1, 3, 32]),
        binary(&[131, 77, 0, 0, 0, 2, 4, 1, 32]),
        binary(&[131, 77, 0, 0, 0, 2, 3, 255, 224]),
        binary(&nested),
        binary(&[
            131, 80, 0, 0, 0, 132, 120, 156, 243, 101, 96, 96, 168, 99, 102, 24, 72, 160, 0, 0,
            104, 213, 0, 239,
        ]),
        binary(&nested),
        binary(&[131, 77, 0, 0, 0, 6, 4, 1, 2, 3, 4, 5, 96]),
    ]);
    let badarg = Term::tuple(vec![atom(&program, "error"), atom(&program, "badarg")]);
    let expected = Term::tuple(vec![
        encoded,
        Term::Integer(9),
        Term::list_from(vec![atom(&program, "true"); 7]),
        Term::list_from(vec![badarg; 4]),
    ]);
    assert_eq!(run(&program, "bits"), expected);
}

/// Return the elements of the proper list `term`.
fn elements(term: &Term) -> Vec<Term> {
    term.proper_list()
        .expect("the result is a proper list")
        .cloned()
        .collect()
}

/// Return the elements of the tuple `term`.
fn fields(term: &Term) -> Vec<Term> {
    let Term::Tuple(fields) = term else {
        panic!("the value is a tuple");
    };
    fields.to_vec()
}

#[test]
fn encodes_each_term_type_as_pinned_erts() {
    // The bytes of `term_to_binary(T)` for each value of
    // `external_term_probe:values/0`, measured on the pinned ERTS with
    // `mise run erts-eval --pa fixtures/external_term`.
    let expected = Term::list_from(vec![
        binary(&[131, 97, 0]),
        binary(&[131, 97, 255]),
        binary(&[131, 98, 0, 0, 1, 0]),
        binary(&[131, 98, 255, 255, 255, 255]),
        binary(&[131, 110, 6, 0, 0, 0, 0, 0, 0, 1]),
        binary(&[131, 110, 6, 1, 0, 0, 0, 0, 0, 1]),
        binary(&[131, 110, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 64]),
        binary(&[131, 110, 9, 1, 0, 0, 0, 0, 0, 0, 0, 0, 64]),
        binary(&[131, 70, 63, 248, 0, 0, 0, 0, 0, 0]),
        binary(&[131, 70, 0, 0, 0, 0, 0, 0, 0, 0]),
        binary(&[131, 70, 128, 0, 0, 0, 0, 0, 0, 0]),
        binary(&[131, 119, 3, 102, 111, 111]),
        binary(&[131, 119, 2, 195, 169]),
        binary(&[131, 106]),
        binary(&[131, 107, 0, 3, 1, 2, 3]),
        binary(&[
            131, 108, 0, 0, 0, 4, 97, 1, 97, 2, 97, 3, 98, 0, 0, 1, 44, 106,
        ]),
        binary(&[131, 108, 0, 0, 0, 1, 97, 1, 97, 2]),
        binary(&[131, 104, 0]),
        binary(&[131, 104, 2, 119, 1, 97, 97, 1]),
        binary(&[
            131, 107, 0, 40, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
            21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40,
        ]),
        binary(&[131, 116, 0, 0, 0, 2, 119, 1, 98, 97, 2, 119, 1, 97, 97, 1]),
        binary(&[131, 109, 0, 0, 0, 3, 97, 98, 99]),
        binary(&[
            131, 113, 119, 6, 101, 114, 108, 97, 110, 103, 119, 3, 97, 98, 115, 97, 1,
        ]),
    ]);
    assert_eq!(run(&load(), "literals"), expected);
}

#[test]
fn runs_term_to_binary_options_as_pinned_erts() {
    // The bytes of `term_to_binary/2` with each option, measured on the
    // pinned ERTS. The compressed form of the list of 40 atoms keeps the
    // zlib stream, and `{compressed, 0}` keeps the plain form
    // (`external.c:2280-2300`). The local term holds the block hash of the
    // node, so the test checks its shape and its decoded value.
    let program = load();
    let mut values = fields(&run(&program, "options"));
    let decoded = values.pop().expect("the decoded local term");
    let local = values.pop().expect("the local bytes");
    let exact = elements(&values.pop().expect("the exact bytes"));
    assert_eq!(
        exact[0..2],
        [
            binary(&[
                131, 80, 0, 0, 0, 206, 120, 156, 203, 97, 96, 96, 208, 40, 103, 78, 76, 74, 30, 6,
                68, 22, 0, 54, 30, 65, 255,
            ]),
            binary(&[
                131, 80, 0, 0, 0, 206, 120, 218, 203, 97, 96, 96, 208, 40, 103, 78, 76, 74, 30, 6,
                68, 22, 0, 54, 30, 65, 255,
            ]),
        ]
    );
    let Term::Binary(plain) = &exact[2] else {
        panic!("the compressed level 0 option gives a binary");
    };
    assert_eq!(&plain.bytes().unwrap()[..6], [131, 108, 0, 0, 0, 40]);
    assert_eq!(plain.bytes().unwrap().len(), 207);
    assert_eq!(exact[3], binary(&[131, 100, 0, 1, 233]));
    assert_eq!(
        exact[4],
        binary(&[
            131, 99, 49, 46, 53, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48, 48,
            48, 48, 48, 101, 43, 48, 48, 0, 0, 0, 0, 0,
        ])
    );
    assert_eq!(
        exact[5],
        binary(&[131, 116, 0, 0, 0, 2, 119, 1, 97, 97, 2, 119, 1, 98, 97, 1])
    );
    let Term::Binary(local) = &local else {
        panic!("the local option gives a binary");
    };
    assert_eq!(&local.bytes().unwrap()[..2], [131, 121]);
    assert_eq!(
        &local.bytes().unwrap()[6..],
        [104, 2, 119, 1, 120, 109, 0, 0, 0, 2, 1, 2]
    );
    assert_eq!(local.bytes().unwrap().len(), 18);
    assert_eq!(
        decoded,
        Term::tuple(vec![atom(&program, "x"), binary(&[1, 2])])
    );
}

#[test]
fn decodes_each_term_type_as_pinned_erts() {
    // `binary_to_term(term_to_binary(T)) =:= T` for each value, the
    // compressed form, `[safe]`, `[local]`, and an external fun, measured
    // on the pinned ERTS.
    let program = load();
    let yes = atom(&program, "true");
    let mut expected = vec![Term::list_from(std::iter::repeat_n(yes.clone(), 23))];
    expected.extend(std::iter::repeat_n(yes, 3));
    expected.push(Term::Integer(5));
    assert_eq!(run(&program, "decode"), Term::list_from(expected));
}

#[test]
fn returns_used_and_external_size_as_pinned_erts() {
    let program = load();

    // `used` counts the bytes of the term and its header, and skips the
    // trailing bytes (`external.c:2090-2120`).
    let expected = Term::list_from(vec![
        Term::tuple(vec![
            Term::tuple(vec![atom(&program, "a"), Term::Integer(1)]),
            Term::Integer(8),
            Term::Integer(8),
        ]),
        Term::tuple(vec![
            Term::list_from(std::iter::repeat_n(atom(&program, "abc"), 200)),
            Term::Integer(32),
            Term::Integer(32),
        ]),
    ]);
    assert_eq!(run(&program, "used"), expected);

    // `external_size/1` gives the size of `term_to_binary/1`
    // (`external.c:2177-2214`).
    let sizes: Vec<Term> = [
        3, 3, 6, 6, 10, 10, 13, 13, 10, 10, 10, 6, 5, 2, 7, 18, 10, 3, 8, 44, 16, 9, 17, 112,
    ]
    .into_iter()
    .map(|size| Term::tuple(vec![Term::Integer(size), Term::Integer(size)]))
    .collect();
    assert_eq!(run(&program, "sizes"), Term::list_from(sizes));
}

#[test]
fn raises_badarg_as_pinned_erts() {
    // Each error case of `errors/0` gives `error:badarg` on the pinned
    // ERTS: a non-binary, a bad version, a truncated term, an invalid tag,
    // an infinity float, bad compression, and each bad option list
    // (`external.c:1750-1760`, `2165-2194`, `4525-4535`, `4651-4665`).
    let program = load();
    let badarg = Term::tuple(vec![atom(&program, "error"), atom(&program, "badarg")]);
    assert_eq!(
        run(&program, "errors"),
        Term::list_from(std::iter::repeat_n(badarg, 19))
    );

    // A term that PBeam cannot hold stops the run with a runtime error.
    let error = program
        .execute("external_term_probe", "decode_record", &[])
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("unsupported erlang:binary_to_term/1,2"),
        "{error}"
    );
}

const FUN_PROBE: &[u8] = include_bytes!("../../fixtures/external_term/fun_ext_probe.beam");

/// The module MD5 of `fun_ext_probe.beam`, which the pinned ERTS writes in
/// each `NEW_FUN_EXT` of the module.
const FUN_PROBE_MD5: [u8; 16] = [
    77, 47, 206, 122, 4, 233, 94, 226, 67, 58, 214, 118, 101, 166, 226, 53,
];

/// Return the program of `fun_ext_probe` with the pinned `erlang` module.
fn load_fun_probe() -> BeamProgram {
    let erlang = pinned_beam("erlang");
    BeamProgram::load(&[&erlang, FUN_PROBE]).unwrap()
}

/// Return the `NEW_FUN_EXT` bytes of the pinned ERTS for a fun of
/// `fun_ext_probe`: the size, the arity, the index, the free count, the old
/// index, and the old unique value `[98, 2, 105, 126, 115]`, then `free`.
fn probe_fun_bytes(size: u8, arity: u8, index: u8, free_count: u8, free: &[u8]) -> Vec<u8> {
    let mut bytes = vec![131, 112, 0, 0, 0, size, arity];
    bytes.extend_from_slice(&FUN_PROBE_MD5);
    bytes.extend_from_slice(&[0, 0, 0, index, 0, 0, 0, free_count, 119, 13]);
    bytes.extend_from_slice(b"fun_ext_probe");
    bytes.extend_from_slice(&[97, index, 98, 2, 105, 126, 115, 88, 119, 13]);
    bytes.extend_from_slice(b"nonode@nohost");
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(free);
    bytes
}

#[test]
fn encodes_and_decodes_local_funs_as_pinned_erts() {
    // `fun_ext_probe:round_trip/0` on the pinned ERTS (OTP 29.1.1): the
    // bytes of `enc_term` (`external.c:3966-3993`), the external size, the
    // equality and the fun information of the decoded fun, and a call of
    // each decoded fun (`dec_term`, `external.c:5102-5176`).
    let program = load_fun_probe();
    let atom = |name| atom(&program, name);
    let tagged = |name, value| Term::tuple(vec![atom(name), value]);
    let entry = |bytes: Vec<u8>, size, index, arity| {
        Term::tuple(vec![
            binary(&bytes),
            Term::Integer(size),
            atom("true"),
            tagged("index", Term::Integer(index)),
            tagged("new_uniq", binary(&FUN_PROBE_MD5)),
            tagged("arity", Term::Integer(arity)),
        ])
    };
    let free = [104, 2, 119, 3, b't', b'a', b'g', 109, 0, 0, 0, 2, 1, 2];
    let tag = Term::tuple(vec![atom("tag"), binary(&[1, 2])]);
    assert_eq!(
        program
            .execute("fun_ext_probe", "round_trip", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            entry(probe_fun_bytes(93, 2, 0, 1, &free), 95, 0, 2),
            entry(probe_fun_bytes(79, 0, 1, 0, &[]), 81, 1, 0),
            Term::tuple(vec![tag, Term::Integer(1), Term::Integer(2)]),
            atom("ok"),
        ])
    );
}

#[test]
fn decodes_a_fun_of_a_module_that_is_not_loaded_as_pinned_erts() {
    // `fun_ext_probe:unloaded/0` on the pinned ERTS: the decode makes a stub
    // fun entry with the decoded values (`erl_fun.c:299-309`), and a new
    // encode writes its size (`external.c:3350-3370`).
    let program = load_fun_probe();
    let result = program
        .execute("fun_ext_probe", "unloaded", &[])
        .unwrap()
        .term;
    let atom = |name| atom(&program, name);
    let tagged = |name, value| Term::tuple(vec![atom(name), value]);
    let mut uniq = [0; 16];
    uniq[15] = 7;
    let mut bytes = vec![131, 112, 0, 0, 0, 75, 1];
    bytes.extend_from_slice(&uniq);
    bytes.extend_from_slice(&[0, 0, 0, 3, 0, 0, 0, 1, 119, 7]);
    bytes.extend_from_slice(b"no_such");
    bytes.extend_from_slice(&[97, 2, 98, 255, 255, 255, 255, 88, 119, 13]);
    bytes.extend_from_slice(b"nonode@nohost");
    bytes.extend_from_slice(&[0; 12]);
    bytes.extend_from_slice(&[97, 5]);
    let info = vec![
        tagged("index", Term::Integer(2)),
        tagged("new_index", Term::Integer(3)),
        tagged("new_uniq", binary(&uniq)),
        tagged("uniq", Term::Integer(-1)),
        tagged("arity", Term::Integer(1)),
        tagged("env", Term::list_from(vec![Term::Integer(5)])),
        tagged("type", atom("local")),
    ];
    assert_eq!(
        result,
        Term::tuple(vec![atom("true"), Term::list_from(info), binary(&bytes)])
    );

    // ERTS calls `error_handler:undefined_lambda/3`, which needs a code
    // server; PBeam stops the run with a runtime error.
    let error = program
        .execute("fun_ext_probe", "call_unloaded", &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing BEAM module no_such"), "{error}");
}

#[test]
fn rejects_bad_new_fun_ext_fields_as_pinned_erts() {
    // `fun_ext_probe:errors/0` on the pinned ERTS: a module that is not an
    // atom, an old index or an old unique value that is not small, a creator
    // that is not a pid, and a truncated fun give badarg; a small old index
    // as a big wraps to a C `int` (`external.c:5130-5167`).
    let program = load_fun_probe();
    let badarg = Term::tuple(vec![atom(&program, "error"), atom(&program, "badarg")]);
    let mut expected = vec![badarg; 5];
    expected.push(Term::tuple(vec![atom(&program, "index"), Term::Integer(1)]));
    assert_eq!(
        program
            .execute("fun_ext_probe", "errors", &[])
            .unwrap()
            .term,
        Term::list_from(expected)
    );
}
