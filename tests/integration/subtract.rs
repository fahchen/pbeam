use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const SUBTRACT_PROBE: &[u8] = include_bytes!("../../fixtures/subtract/subtract_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// large heap, so that no measured process collects garbage. PBeam has no
/// process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `subtract_probe:results/1` on the pinned ERTS with `erl +S 1` (JIT): the
/// result summary and the reductions of each case.
const ERTS_RESULTS: &str = "[{nil_nil,[],2},{nil_list,[],2},{list_nil,[1,2,3],2},\
    {naive_left,[2,4,5],3},\
    {naive_right,[1,2,4,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20],4},\
    {naive_right_used,[1,3,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20],2},\
    {set,{59,1,40},15},\
    {set_used,[1,2,3,4,5,6,7,8,9,31,32,33,34,35,36,37,38,39,40],11},\
    {set_terms,[b,2],6},{length_left_trap,{200000,7,7},18752},\
    {naive_left_trap,[2,3,4,5,6,7,8,9,10],11251},\
    {length_right_trap,[2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20],18754},\
    {naive_right_trap,{199999,7,7},118738},{set_build_trap,[],9379},\
    {set_finish_trap,{199999,7,7},31253},{not_list_left,{error,badarg},2},\
    {not_list_right,{error,badarg},2},{improper_left,{error,badarg},2},\
    {improper_right,{error,badarg},2},{improper_naive_left,{error,badarg},2},\
    {improper_after_trap,{error,badarg},1252},{improper_right_long,{error,badarg},2}]";

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
fn subtracts_lists_in_each_stage_as_pinned_erts() {
    // `erlang:'--'/2` counts the left list, then removes each right element
    // from at most 16 left elements, or counts the right list and keeps the
    // left elements that at most 16 right elements, or a multiset of more
    // right elements, do not remove (erl_bif_lists.c:931-1000). Each stage
    // traps when its budget ends, and an improper list raises `badarg`
    // when a stage reaches its tail. The reductions of each case are those
    // of the pinned ERTS JIT.
    let program = BeamProgram::load(&[SUBTRACT_PROBE]).unwrap();
    let results = program
        .execute(
            "subtract_probe",
            "results",
            &[program.host_term(Term::nil())],
        )
        .unwrap();
    assert_eq!(text(&results), ERTS_RESULTS);

    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/subtract",
            "-eval",
            &format!(
                "jit = erlang:system_info(emu_flavor), \
                 io:format(\"~w~n\", [subtract_probe:results({ERTS_OPTIONS})]), halt()."
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
