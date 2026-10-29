//! Runs the `adler32` and `crc32` BIFs on PBeam and compares the results with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/chksum/chksum_probe.beam");

/// `chksum_probe:main()` on the pinned ERTS with `erl +S 1`, as
/// `io:format("~w")` prints it: the sums, the combined sums, the `badarg`
/// cases, and the reductions of two calls.
const ERTS_RESULT: &str = "[[1,0,38600999,891568578,64487819,1438416925,47513716,1493393559,917518,4294967295,1900559,4325396,826628592,372159703,338100908,75363461,441418284,891803698,1795752698,1182927836],[7798796,1,1501836662,3,176592708,true],[{error,badarg,{erlang,crc32,2}},{error,badarg,{erlang,adler32,2}},{error,badarg,{erlang,adler32,2}},{error,badarg,{erlang,crc32,2}},{error,badarg,{erlang,crc32,1}},{error,badarg,{erlang,adler32,1}},{error,badarg,{erlang,adler32,1}},{error,badarg,{erlang,crc32,1}},{error,badarg,{erlang,crc32,2}},{error,badarg,{erlang,adler32_combine,3}},{error,badarg,{erlang,adler32_combine,3}},{error,badarg,{erlang,crc32_combine,3}}],[106,1109]]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe result.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::nil()]), &atoms), "[[]]");
/// ```
fn text(term: &Term, atoms: &AtomTable) -> String {
    let join = |terms: &[Term]| {
        terms
            .iter()
            .map(|term| text(term, atoms))
            .collect::<Vec<_>>()
            .join(",")
    };
    match term {
        Term::Atom(atom) => atoms.name_of(atom).to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn runs_the_adler32_and_crc32_bifs_as_pinned_erts() {
    // `chksum_probe:main/0` calls the BIFs with `call_ext` and catches the
    // `badarg` cases (`erl_bif_chksum.c:53-451`). The probe wraps each
    // argument in `id/1`, so the compiler folds no constant into the call.
    let program = BeamProgram::load(&[PROBE]).unwrap();
    let result = program.execute("chksum_probe", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_RESULT);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "+S",
            "1",
            "-pa",
            "fixtures/chksum",
            "-eval",
            "io:format(\"~w~n\", [chksum_probe:main()]), halt().",
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{ERTS_RESULT}\n")
    );
}
