use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const FROM_KEYS_PROBE: &[u8] = include_bytes!("../../fixtures/from_keys/from_keys_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// large heap, so that no measured process collects garbage. PBeam has no
/// process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `from_keys_probe:results/1` on the pinned ERTS with `erl +S 1` (JIT): the
/// map size, whether the map equals the map of `maps:from_list/1`, and the
/// reductions of each case.
const ERTS_RESULTS: &str = "[{empty,{0,true},2},{flat,{3,true},2},{flat_limit,{32,true},2},\
    {dups,{1,true},10},{more_dups,{1,true},101},{trap,{1,true},50001},\
    {two_keys,{2,true},25},{two_keys_again,{2,true},25},{improper,{error,badarg},2},\
    {not_list,{error,badarg},2}]";

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe results.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::list_from(vec![Term::Integer(1)])), "[1]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        _ if let Some(terms) = term.proper_list() => {
            format!("[{}]", join(&terms.cloned().collect::<Vec<_>>()))
        }
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn makes_maps_from_keys_as_pinned_erts() {
    // `maps:from_keys/2` walks the keys with one YCF unit for each element,
    // builds a flatmap for at most 32 keys, and else builds a hashmap with
    // the units of `maps:from_list/1`, 40 units for each reduction
    // (erl_map.c:279-311, 482-560, 629-642). `code_server` makes the set of
    // the files of a code path directory with it (code_server.erl:1146-1159).
    let program = BeamProgram::load(&[FROM_KEYS_PROBE]).unwrap();
    let results = program
        .execute(
            "from_keys_probe",
            "results",
            &[program.host_term(Term::nil())],
        )
        .unwrap();
    assert_eq!(text(&results.term), ERTS_RESULTS);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/from_keys",
            "-eval",
            &format!(
                "jit = erlang:system_info(emu_flavor), \
                 io:format(\"~w~n\", [from_keys_probe:results({ERTS_OPTIONS})]), halt()."
            ),
        ])
        .output()
        .unwrap();
    assert!(
        reference.status.success() && reference.stderr.is_empty(),
        "{reference:?}"
    );
    assert_eq!(
        String::from_utf8(reference.stdout).unwrap(),
        format!("{ERTS_RESULTS}\n")
    );
}

#[test]
fn stops_maps_from_keys_that_finds_an_improper_tail_after_a_trap() {
    // The walk of 200000 keys yields before it reaches the tail. ERTS then
    // returns from `BIF_ERROR` before YCF clears its trap state, and traps
    // again forever (utils.c:3757-3773).
    let program = BeamProgram::load(&[FROM_KEYS_PROBE]).unwrap();
    let failure = program
        .execute("from_keys_probe", "improper_after_trap", &[])
        .unwrap_err()
        .to_string();
    assert!(
        failure.contains("maps:from_keys/2 finds an improper tail after a trap"),
        "{failure}"
    );
}
