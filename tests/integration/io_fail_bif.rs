//! Runs the Unicode IO BIFs (`characters_to_list` and related) on PBeam and compares them with the
//! pinned ERTS.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

use common::pinned_beam;
use pbeam_runtime::{BeamProgram, Term};

const IO_FAIL_BIF_PROBE: &[u8] =
    include_bytes!("../../fixtures/io_fail_bif/io_fail_bif_probe.beam");

/// Run `io_fail_bif_probe:Function()` on the pinned ERTS and return its `~w`
/// output.
fn reference(function: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/io_fail_bif",
            "-eval",
            &format!("io:format(\"~w~n\", [io_fail_bif_probe:{function}()]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert_eq!(reference.stderr, [] as [u8; 0]);
    reference.stdout
}

#[test]
fn checks_unicode_io_of_a_pid_as_pinned_erts() {
    // `io_fail_bif_probe:unicode_io/0` runs `net_kernel:dflag_unicode_io/1`,
    // as `io:bc_req/3` does for each `io:format/3` call to a pid. A pid of
    // the local node gives `true`, also after the process ends, and a term
    // that is not a pid raises `badarg` (`net_kernel_dflag_unicode_io_1`,
    // `dist.c:6670-6686`). PBeam has only local pids.
    let program = BeamProgram::load(&[IO_FAIL_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let badarg = Term::tuple(vec![atom("error"), atom("badarg")]);
    assert_eq!(
        program
            .execute("io_fail_bif_probe", "unicode_io", &[])
            .unwrap()
            .term,
        Term::list_from(vec![atom("true"), atom("true"), badarg.clone(), badarg])
    );
    assert_eq!(
        reference("unicode_io"),
        b"[true,true,{error,badarg},{error,badarg}]\n"
    );
}

#[test]
fn converts_chardata_to_a_list_as_pinned_erts() {
    // `io_fail_bif_probe:to_list/0` runs `unicode:characters_to_list/2`, as
    // `standard_error` does for each `put_chars` request
    // (standard_error.erl:266-287), and catches each exception. The cases
    // are the binary path, the list path, kept bytes of a sequence that
    // continues in the next binary, the `Rest` form of each list that the
    // walk entered, and the shape check of `utf8_need`
    // (erl_unicode.c:237-340, 342-505, 508-749, 751-771, 1122-1283).
    let program = BeamProgram::load(&[IO_FAIL_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let list = |code_points: &[i64]| Term::list_from(code_points.iter().copied().map(int));
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let stopped =
        |reason, converted: &[i64], rest| Term::tuple(vec![atom(reason), list(converted), rest]);
    let error = |converted, rest| stopped("error", converted, rest);
    let incomplete = |converted, rest| stopped("incomplete", converted, rest);
    let badarg = || Term::tuple(vec![atom("error"), atom("badarg")]);
    let improper = |elements, tail| Term::list_with_tail(elements, tail);
    assert_eq!(
        program
            .execute("io_fail_bif_probe", "to_list", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            list(&[72, 101, 108, 108, 111]),
            error(&[97], binary(&[255, 98])),
            incomplete(&[97], binary(&[195])),
            incomplete(&[97], binary(&[237, 160])),
            incomplete(&[97], binary(&[224, 128])),
            incomplete(&[97, 98], binary(&[195])),
            error(&[97], list(&[55296, 98])),
            error(&[97], list(&[-1])),
            badarg(),
            badarg(),
            badarg(),
            list(&[200]),
            error(&[], list(&[256])),
            error(&[200], list(&[300])),
            list(&[195, 169]),
            list(&[233]),
            list(&[233]),
            error(&[], Term::list_from(vec![binary(&[195]), list(&[97])])),
            incomplete(&[97], binary(&[195])),
            incomplete(&[97], binary(&[195])),
            error(&[97], Term::list_from(vec![binary(&[255]), int(98)])),
            error(
                &[97, 98],
                Term::list_from(vec![
                    Term::list_from(vec![list(&[1_114_112, 99])]),
                    int(100)
                ])
            ),
            error(&[], list(&[1 << 60])),
            error(&[], improper(vec![int(1 << 60)], atom("foo"))),
            incomplete(&[], binary(&[240, 159, 152])),
            incomplete(&[], binary(&[240, 159, 152])),
            error(&[], binary(&[240, 159, 65])),
            incomplete(&[120], binary(&[224, 160])),
            error(&[], binary(&[224, 97])),
            list(&[]),
            list(&[]),
            list(&[120]),
            list(&[97, 233]),
            error(
                &[],
                Term::list_from(vec![
                    binary(&[195]),
                    Term::list_from(vec![binary(&[65, 66])])
                ])
            ),
            incomplete(&[97], binary(&[192])),
            error(&[97], Term::list_from(vec![binary(&[255]), int(1)])),
            error(&[], improper(vec![list(&[-1])], binary(b"x"))),
            list(&[128_512, 97]),
            error(&[], binary(&[128])),
        ])
    );
    assert_eq!(
        reference("to_list"),
        b"[[72,101,108,108,111],{error,[97],<<255,98>>},{incomplete,[97],<<195>>},\
{incomplete,[97],<<237,160>>},{incomplete,[97],<<224,128>>},{incomplete,[97,98],<<195>>},\
{error,[97],[55296,98]},{error,[97],[-1]},{error,badarg},{error,badarg},{error,badarg},\
[200],{error,[],[256]},{error,[200],[300]},[195,169],[233],[233],{error,[],[<<195>>,[97]]},\
{incomplete,[97],<<195>>},{incomplete,[97],<<195>>},{error,[97],[<<255>>,98]},\
{error,[97,98],[[[1114112,99]],100]},{error,[],[1152921504606846976]},\
{error,[],[1152921504606846976|foo]},{incomplete,[],<<240,159,152>>},\
{incomplete,[],<<240,159,152>>},{error,[],<<240,159,65>>},{incomplete,[120],<<224,160>>},\
{error,[],<<224,97>>},[],[],[120],[97,233],{error,[],[<<195>>,[<<65,66>>]]},\
{incomplete,[97],<<192>>},{error,[97],[<<255>>,1]},{error,[],[[-1]|<<120>>]},[128512,97],\
{error,[],<<128>>}]\n"
    );
}

#[test]
fn converts_chardata_of_100000_characters_as_pinned_erts() {
    // `io_fail_bif_probe:big/0` converts inputs of 100000 characters, which
    // ERTS converts in more than one call (`allowed_iterations`,
    // erl_unicode.c:166-174). PBeam converts each in one call, and the
    // `Rest` is the same as for a short input. Each result is
    // `{Tag, length(Converted), Rest}`.
    let program = BeamProgram::load(&[IO_FAIL_BIF_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let int = Term::Integer;
    let binary = |bytes: &[u8]| Term::binary(bytes.to_vec());
    let summary = |reason, length, rest| Term::tuple(vec![atom(reason), int(length), rest]);
    assert_eq!(
        program
            .execute("io_fail_bif_probe", "big", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            summary(
                "error",
                100_000,
                Term::list_from(vec![Term::list_from(vec![int(-1), int(98)])])
            ),
            summary(
                "error",
                100_002,
                Term::list_from(vec![
                    Term::list_from(vec![
                        Term::list_from(vec![Term::list_from(vec![binary(&[255]), int(1)])]),
                        int(2),
                    ]),
                    int(3),
                ])
            ),
            summary("error", 100_000, binary(&[255, 1])),
            summary(
                "error",
                100_000,
                Term::list_from(vec![binary(&[195]), Term::list_from(vec![int(7)])])
            ),
            summary(
                "error",
                100_000,
                Term::list_from(vec![
                    Term::list_from(vec![Term::list_from(vec![int(-1), int(2)]), int(3)]),
                    int(4),
                ])
            ),
            summary("incomplete", 100_000, binary(&[240, 159])),
        ])
    );
    assert_eq!(
        reference("big"),
        b"[{error,100000,[[-1,98]]},{error,100002,[[[[<<255>>,1]],2],3]},{error,100000,<<255,1>>},\
{error,100000,[<<195>>,[7]]},{error,100000,[[[-1,2],3],4]},{incomplete,100000,<<240,159>>}]\n"
    );
}

#[test]
fn traps_another_encoding_to_characters_to_list_int() {
    // ERTS traps each encoding other than `latin1`, `unicode`, and `utf8`
    // to `unicode:characters_to_list_int/2` (erl_unicode.c:1135-1143),
    // which calls `unicode:characters_to_binary/2` with the same encoding
    // (unicode.erl:840-857). PBeam does not convert `utf16` there (row
    // "Chardata conversion" in docs/erts-differences.md), so the run stops
    // in the pinned `unicode` module, which shows that the trap ran.
    let unicode = pinned_beam("unicode");
    let program = BeamProgram::load(&[&unicode, IO_FAIL_BIF_PROBE]).unwrap();
    assert_eq!(
        program
            .execute("io_fail_bif_probe", "trap", &[])
            .unwrap_err()
            .to_string(),
        "unsupported unicode:characters_to_binary/2 encoding Atom(\"utf16\")"
    );
    assert_eq!(reference("trap"), b"[97,98,99]\n");
}
