use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

const PROBE: &[u8] = include_bytes!("../../fixtures/hash/hash_probe.beam");

/// `hash_probe:main()` on the pinned ERTS with `erl +S 1`, as
/// `io:format("~w")` prints it: the md5 groups, the phash groups, and the
/// reductions of one md5 call and two phash2 calls.
const ERTS_RESULT: &str = "[[[212,29,140,217,143,0,178,4,233,128,9,152,236,248,66,126],[212,29,140,217,143,0,178,4,233,128,9,152,236,248,66,126],[144,1,80,152,60,210,79,176,214,150,63,125,40,225,127,114],[82,137,223,115,125,245,115,38,252,221,34,89,122,251,31,172],[82,137,223,115,125,245,115,38,252,221,34,89,122,251,31,172],[132,22,234,254,31,229,154,25,87,172,104,3,106,209,156,203],[152,197,221,113,159,191,200,55,146,249,217,241,138,5,11,247],[152,197,221,113,159,191,200,55,146,249,217,241,138,5,11,247],[152,197,221,113,159,191,200,55,146,249,217,241,138,5,11,247],{error,badarg,[{erlang,md5,1}]},{error,badarg,[{erlang,md5,1}]},{error,badarg,[{erlang,md5,1}]}],[96,[1,35,69,103,137,171,205,239,254,220,186,152,118,84,50,16,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],96,false,[144,1,80,152,60,210,79,176,214,150,63,125,40,225,127,114],[144,1,80,152,60,210,79,176,214,150,63,125,40,225,127,114],[212,29,140,217,143,0,178,4,233,128,9,152,236,248,66,126],[232,11,80,23,9,137,80,252,88,170,216,60,140,20,151,142],96,[176,230,65,201,152,204,62,174,111,162,248,114,109,152,205,221],{error,badarg,[{erlang,md5_final,1}]},{error,badarg,[{erlang,md5_update,2}]},{error,badarg,[{erlang,md5_update,2}]},{error,badarg,[{erlang,md5_final,1}]},{error,badarg,[{erlang,md5_final,1}]},{error,badarg,[{erlang,md5_update,2}]}],[[2147483673,26969617,2442888721,1,3,0,1],[2788898428,103122609,2519041713,1,1,0,0],[2147489129,35173814,2316875190,1,2,0,0],[1680185270,13457787,2563594619,1,2,0,2],[1875852311,53443540,321878996,1,2,0,2],[1073217537,10380315,2023646235,1,3,0,0],[1756186964,44786219,313221675,1,2,0,0],[3336869158,25788620,4186538188,1,1,0,1],[1670235874,59617982,1401795262,1,1,0,1],[687692590,6479071,4167228639,1,1,0,0],[26500,26499,26499,1,1,0,0],[1680185270,44071773,1117813597,1,2,0,1],[1,88723725,3175731469,1,1,0,1],[2788898428,2614250,539485162,1,1,0,1],[2,113427502,3468870702,1,2,0,0],[1,87486268,221703996,1,1,0,0],[1,13708901,147926629,1,1,0,1],[1516930873,40530709,1248490261,1,1,0,1],[1272196176,43840879,1251800431,1,3,0,1],[2197141286,70365907,2217849555,1,2,0,0],[3560546145,1451227,2954241243,1,3,0,0],[2470972762,55048502,2202532150,1,1,0,1],[84150055,32453756,569324668,1,1,0,1],[466678365,32295536,3253521008,1,3,0,2],[4079174615,123609408,2405310784,1,2,0,1],[4079174615,123609408,2405310784,1,2,0,1],[2573105746,51683826,2467602930,1,1,0,0],[2184021715,116490133,2129756053,1,1,0,1],[796953231,39846312,576717224,1,3,0,2],{error,badarg,[{erlang,phash,2}]},{error,badarg,[{erlang,phash,2}]},{error,badarg,[{erlang,phash,2}]},{error,badarg,[{erlang,phash,2}]},{error,badarg,[{erlang,phash,2}]},{error,badarg,[{erlang,phash,2}]},{error,badarg,[{erlang,phash2,2}]},{error,badarg,[{erlang,phash2,2}]},{error,badarg,[{erlang,phash2,2}]},{error,badarg,[{erlang,phash2,2}]},{error,badarg,[{erlang,phash2,2}]},{error,badarg,[{erlang,phash2,2}]}],[1006,8,7]]";

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
        Term::BigInt(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn runs_the_md5_and_phash_bifs_as_pinned_erts() {
    // `hash_probe:main/0` calls the BIFs with `call_ext` and catches the
    // `badarg` cases (`erl_bif_chksum.c:455-610`, `bif.c:5319-5402`). The
    // probe wraps each argument in `id/1`, so the compiler folds no constant
    // into the call.
    let program = BeamProgram::load(&[PROBE]).unwrap();
    let result = program.execute("hash_probe", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_RESULT);

    let reference = Command::new("erl")
        .args([
            "-noshell",
            "+S",
            "1",
            "-pa",
            "fixtures/hash",
            "-eval",
            "io:format(\"~w~n\", [hash_probe:main()]), halt().",
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
