use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/erlang_bifs/erlang_bifs_probe.beam");

/// `erlang_bifs_probe:main()` on the pinned ERTS, as `io:format("~w")`
/// prints it: `integer_to_list/2`, the tuple BIFs, the rounding BIFs, the
/// term text BIFs, and the clock BIFs.
///
/// Each caught error holds the class, the reason, and the first frame of
/// the stack trace: the frame of the BIF with its arity, which ERTS
/// `save_stacktrace` puts first (`beam_common.c:880-1055`). The values of
/// the clock BIFs vary between the runs, so the probe gives their shape.
const ERTS_RESULT: &str = "[[[70,70],[45,49,49,49,49,49,49,49,49],[90],[45,49],[56,48,48,48,48,48,48,48,48,48,48,48,48,48,48],{error,badarg,[{erlang,integer_to_list,2}]},{error,badarg,[{erlang,integer_to_list,2}]},{error,badarg,[{erlang,integer_to_list,2}]},{error,badarg,[{erlang,integer_to_list,2}]},{error,badarg,[{erlang,integer_to_list,2}]},{error,badarg,[{erlang,integer_to_list,2}]},{error,badarg,[{erlang,integer_to_list,2}]}],[{c,b},{a,c},{c,a,b},{a,b,c},{b},{},{},{a,a},{x,x},{a,b,x},{c,b},{a,b},{error,badarg,[{erlang,setelement,3}]},{error,badarg,[{erlang,setelement,3}]},{error,badarg,[{erlang,setelement,3}]},{error,badarg,[{erlang,setelement,3}]},{error,badarg,[{erlang,setelement,3}]},{error,badarg,[{erlang,insert_element,3}]},{error,badarg,[{erlang,insert_element,3}]},{error,badarg,[{erlang,insert_element,3}]},{error,badarg,[{erlang,delete_element,2}]},{error,badarg,[{erlang,delete_element,2}]},{error,badarg,[{erlang,delete_element,2}]},{error,badarg,[{erlang,make_tuple,2}]},{error,badarg,[{erlang,make_tuple,2}]},{error,badarg,[{erlang,make_tuple,2}]},{error,badarg,[{erlang,make_tuple,2}]},{error,badarg,[{erlang,make_tuple,3}]},{error,badarg,[{erlang,make_tuple,3}]},{error,badarg,[{erlang,make_tuple,3}]},{error,badarg,[{erlang,make_tuple,3}]},{error,badarg,[{erlang,make_tuple,3}]},{error,badarg,[{erlang,make_tuple,3}]},{error,badarg,[{erlang,make_tuple,3}]}],[1,-1,0,0,0,7,-7,576460752303423488,1000000000000000052504760255204420248704468581108159154915854115511802457988908195786371375080447864043704443832883878176942523235360430575644792184786706982848387200926575803737830233794788090059368953234970799945081119038967640880074652742780142494579258788820056842838115669472196386865459400540160,1,-1,2,-2,3,-3,0,7,1,-2,-1,0,-7,2,-1,1,0,-7,-576460752303423488,{error,badarg,[{erlang,trunc,1}]},{error,badarg,[{erlang,round,1}]},{error,badarg,[{erlang,floor,1}]},{error,badarg,[{erlang,ceil,1}]}],[true,true,{error,badarg,[{erlang,pid_to_list,1}]},{error,badarg,[{erlang,pid_to_list,1}]},{error,badarg,[{erlang,port_to_list,1}]},{error,badarg,[{erlang,port_to_list,1}]}],[true,true,true,true,true,true]]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe result.
///
/// The atom names come from `atoms`, the table that owns the term.
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
        Term::BigInt(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn runs_the_small_erlang_bifs_as_pinned_erts() {
    // `erlang_bifs_probe:main/0` calls the BIFs with `call_ext` and catches
    // the `badarg` cases (`bif.c:2976-3180`, `3006-3078`, `3266-3311`,
    // `3341-3374`, `4505-4517`; `erl_bif_guard.c:134-204`;
    // `erl_time_sup.c:2500-2525`). The probe wraps each argument in `id/1`,
    // so the compiler folds no constant into the call.
    let program = BeamProgram::load(&[PROBE]).unwrap();
    let result = program.execute("erlang_bifs_probe", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_RESULT);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/erlang_bifs",
            "-eval",
            "io:format(\"~w~n\", [erlang_bifs_probe:main()]), halt().",
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
