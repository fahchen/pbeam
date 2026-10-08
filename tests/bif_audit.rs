use std::process::Command;

use pbeam_runtime::{BeamProgram, Term};

const BIF_AUDIT_PROBE: &[u8] = include_bytes!("../fixtures/preempt/bif_audit_probe.beam");

/// The spawn options of the measured processes in the ERTS reference run: a
/// large heap, so that no measured process collects garbage. PBeam has no
/// process heap and runs with `[]`.
const ERTS_OPTIONS: &str = "[{min_heap_size,4000000},{min_bin_vheap_size,40000000}]";

/// `bif_audit_probe:costs/1` on the pinned ERTS with `erl +S 1` (JIT), after
/// the persistent terms of the boot are erased: for each BIF of the PBeam
/// BIF table that has a body, the reductions of a call with a small input,
/// and with a large input where the input has a size. Each cost includes
/// the call instruction and the calls of `id/1` that build an argument.
const ERTS_COSTS: &str = "[{times,[0,0]},{plus,[0,0]},{minus,[0,0]},{append,[1,167]},\
    {exact_eq,[0,0]},{exact_ne,[0,0]},{ge,[0,0]},{atom_to_list,[1,1]},{bump_reductions,[11]},\
    {cancel_timer,[1]},{demonitor1,[1]},{demonitor2,[1]},{int_div,[4]},{element,[0,0]},\
    {error1,[1,1]},{error3,[1]},{exit1,[1]},{exit2,[1]},{function_exported,[1]},{get,[0,0]},\
    {group_leader0,[1]},{hd,[0,0]},{integer_to_list,[3]},{iolist_size,[2,1252]},\
    {iolist_to_iovec,[3,2503]},{is_function1,[2]},{is_function2,[2]},{is_map_key,[0,0]},\
    {is_process_alive,[1]},{length,[0,625]},{link,[1]},{list_to_atom,[1,1]},{load_nif,[4]},\
    {make_fun,[0]},{make_ref,[1]},{map_get,[0,0]},{min,[0,0]},{monitor2,[1]},{monitor3,[1]},\
    {monotonic_time,[1]},{nif_error,[1]},{node0,[0]},{node1,[0]},{'not',[2]},\
    {posixtime_to_universaltime,[1]},{process_flag,[1]},{process_info,[1]},{put,[0,0]},\
    {raise,[1]},{register,[1]},{int_rem,[4]},{self,[0]},{send2,[1,313]},{send3,[1]},\
    {spawn,[1,1]},{spawn_link,[1]},{spawn_opt,[1]},{start_timer,[1]},{system_flag,[1]},\
    {system_info,[1]},{tl,[0,0]},{tuple_size,[2]},{universaltime_to_localtime,[1]},\
    {unlink,[1]},{whereis,[1]},{group_leader2,[2]},{group_leader3,[3]},\
    {is_process_alive2,[2]},{map_next,[1]},{perf_counter_unit,[1]},{ets_delete,[1,1]},\
    {ets_insert,[1]},{ets_lookup,[1,1]},{ets_lookup_element,[1,1]},{ets_member,[1,1]},\
    {ets_new,[1]},{ets_whereis,[1]},{printable_range,[1]},{keyfind,[1,1]},{keymember,[1,1]},\
    {member,[2,626]},{reverse,[1,167]},{maps_find,[1,1]},{maps_from_list,[1]},\
    {maps_keys,[3]},{maps_merge,[5]},{maps_remove,[1,1]},{dflag_unicode_io,[1]},\
    {getenv,[1,1]},{set_signal,[1]},{system_time,[1]},{pt_get0,[1]},{pt_get1,[1,1]},\
    {pt_get2,[1]},{pt_put,[2]},{re_compile,[1,1]},{characters_to_binary,[1]},\
    {characters_to_list,[1]}]";

/// The cost that PBeam does not charge yet: a local send of a message of
/// more than 64 heap words bumps one reduction for each 64 words that ERTS
/// copies (`erl_message.c:881-886`, `erl_message.h:33-35`). A list of 10000
/// small integers has 20000 words, so it costs 312 more. The copy counts a
/// shared subterm once and does not copy a literal (`SHCOPY_SEND`,
/// `global.h:1053`), and PBeam terms keep no sharing or literal origin (row
/// "Scheduling" in `docs/erts-differences.md`).
const SEND_COPY: (&str, &str) = ("{send2,[1,313]}", "{send2,[1,1]}");

/// Return a term as `io:format("~w")` prints it, for the atoms, integers,
/// lists, and tuples of the probe results. An atom that is an Erlang
/// keyword is quoted.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(text(&Term::List(vec![Term::Integer(1)])), "[1]");
/// ```
fn text(term: &Term) -> String {
    let join = |terms: &[Term]| terms.iter().map(text).collect::<Vec<_>>().join(",");
    match term {
        Term::Atom(atom) if atom.name() == "not" => "'not'".to_owned(),
        Term::Atom(atom) => atom.name().to_owned(),
        Term::Integer(value) => value.to_string(),
        Term::List(terms) => format!("[{}]", join(terms)),
        Term::Tuple(terms) => format!("{{{}}}", join(terms)),
        other => panic!("unexpected term {other:?}"),
    }
}

#[test]
fn charges_each_bif_with_a_body_as_pinned_erts_except_the_message_copy() {
    let program = BeamProgram::load(&[BIF_AUDIT_PROBE]).unwrap();
    let costs = program
        .execute("bif_audit_probe", "costs", &[Term::List(vec![])])
        .unwrap();
    assert_eq!(text(&costs), ERTS_COSTS.replace(SEND_COPY.0, SEND_COPY.1));

    // The reference run checks that the pinned `erl` is the JIT flavor, and
    // that no measured process collects garbage. It first erases the
    // persistent terms of the boot, because a PBeam run starts with none.
    let reference = Command::new("erl")
        .args([
            "+S",
            "1",
            "-noshell",
            "-pa",
            "fixtures/preempt",
            "-eval",
            &format!(
                "{{module, M}} = code:ensure_loaded(bif_audit_probe), \
                 jit = erlang:system_info(emu_flavor), O = {ERTS_OPTIONS}, \
                 true = erts_internal:erase_persistent_terms(), C = M:costs(O), \
                 [0] = lists:usort([element(3, G) || G <- M:gcs(O)]), \
                 io:format(\"~w~n\", [C]), halt()."
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
        format!("{ERTS_COSTS}\n")
    );
}
