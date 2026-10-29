//! Runs the diagnostics BIFs (display, module delete, `process_info`) on PBeam and compares them
//! with the pinned ERTS.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers report a failure by a panic"
)]

use std::process::Command;

use pbeam_runtime::{AtomTable, BeamProgram, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const PROBE: &[u8] = include_bytes!("../../fixtures/diagnostics/diagnostics_probe.beam");
const TARGET: &[u8] = include_bytes!("../../fixtures/diagnostics/diagnostics_target.beam");
const ON_LOAD: &[u8] = include_bytes!("../../fixtures/diagnostics/diagnostics_on_load.beam");

/// `diagnostics_probe:main()` on the pinned ERTS, as `io:format("~w")`
/// prints it. The probe gives relations for the values that depend on the
/// run: the alias references, the unique integer counters, the scheduler
/// count, and the live process items.
const ERTS_RESULT: &str = "{[registered_name,current_function,initial_call,status,message_queue_len,links,dictionary,trap_exit,error_handler,priority,group_leader,total_heap_size,heap_size,stack_size,reductions,garbage_collection,suspending],[atom,tuple,tuple,atom,integer,list,list,atom,atom,atom,pid,integer,integer,integer,integer,list,list],[true,true],[true,true],[true,true,true,true,false,false],[{error,badarg},{error,badarg},{error,badarg},{error,badarg}],[true,true,true,true,true,true],[{error,badarg},{error,badarg},{error,badarg},{error,badarg}],1000000000,true,[true,true,{error,badarg}],{error,badarg},{true,true,false,undefined,{error,badarg}},diagnostics_handler,[{[current_function,initial_call,status,message_queue_len,links,dictionary,trap_exit,error_handler,priority,group_leader,total_heap_size,heap_size,stack_size,reductions,garbage_collection,suspending],[tuple,tuple,atom,integer,list,list,atom,atom,atom,pid,integer,integer,integer,integer,list,list]},{[current_function,initial_call,status,message_queue_len,links,dictionary,trap_exit,error_handler,priority,group_leader,total_heap_size,heap_size,stack_size,reductions,garbage_collection,suspending],[tuple,tuple,atom,integer,list,list,atom,atom,atom,pid,integer,integer,integer,integer,list,list]}],{true,false,true},undefined}";

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

/// Run an expression on the pinned ERTS with the fixture code path and
/// return its `~w` output.
fn reference(expression: &str) -> String {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/diagnostics",
            "-eval",
            &format!("io:format(\"~w~n\", [{expression}]), halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success(), "{reference:?}");
    String::from_utf8(reference.stdout).unwrap()
}

#[test]
fn runs_the_diagnostics_bifs_as_pinned_erts() {
    // `diagnostics_probe:main/0` calls `erlang:process_info/1`
    // (`erl_bif_info.c:1007-1025`, `1560-1563`), `registered/0`
    // (`register.c:576-604`), `ports/0` (`bif.c:4125-4128`), `alias/0` and
    // `alias/1` (`bif.c:5852-5892`), `unalias/1` (`bif.c:5896-5927`),
    // `unique_integer/1` (`erl_bif_unique.c:840-871`), `system_info/1`,
    // `ref_to_list/1` (`bif.c:4459-4465`), `delete_module/1`
    // (`beam_bif_load.c:679-736`), and `erts_internal:time_unit/0`
    // (`erl_time_sup.c:2537-2540`). `erlang:alias/0` comes from
    // `erlang.beam`.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang, &lists]).unwrap();
    let result = program.execute("diagnostics_probe", "main", &[]).unwrap();
    assert_eq!(text(&result.term, result.atoms()), ERTS_RESULT);
    assert_eq!(
        reference("diagnostics_probe:main()"),
        format!("{ERTS_RESULT}\n")
    );
}

#[test]
fn raises_badarg_for_the_display_string_cases_of_pinned_erts() {
    // `erts_unicode_list_to_buf` (`utils.c:2862-2925`) rejects an element
    // that is not a code point, a surrogate, a code point above 0x10FFFF,
    // and an improper tail, and `display_string_2` (`bif.c:4227-4248`)
    // rejects a device that is not stdout, stderr, or stdin, and a string
    // that is not a list or a binary.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang, &lists]).unwrap();
    let result = program
        .execute("diagnostics_probe", "display_bad", &[])
        .unwrap();
    let expected = "[{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg},{error,badarg}]";
    assert_eq!(text(&result.term, result.atoms()), expected);
    assert_eq!(
        reference("diagnostics_probe:display_bad()"),
        format!("{expected}\n")
    );
}

#[test]
fn raises_badarg_for_a_second_delete_of_a_module() {
    // `delete_module_1` moves the current code to old code
    // (`beam_bif_load.c:2412-2455`). A second delete finds old code and
    // raises `badarg` (`beam_bif_load.c:702-708`). ERTS also writes a
    // logger report before the exception; PBeam has no error report.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang]).unwrap();
    let result = program
        .execute("diagnostics_probe", "delete_old", &[])
        .unwrap();
    assert_eq!(text(&result.term, result.atoms()), "{error,badarg}");
}

#[test]
fn raises_a_runtime_error_for_the_priority_alias_option() {
    // ERTS sets the priority flag of the alias (`bif.c:5869-5870`). PBeam
    // has no priority signals, so the BIF gives a runtime error instead of
    // an alias with a different signal order.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang]).unwrap();
    let error = program
        .execute("diagnostics_probe", "alias_priority", &[])
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("unsupported erlang:alias/1 option priority"),
        "{error}"
    );
}

#[test]
fn reads_prepared_code_on_load_as_pinned_erts() {
    // `erlang:has_prepared_code_on_load/1` (`beam_bif_load.c:268-283`) reads
    // a reference of `erts_internal:prepare_loading/2`
    // (`beam_load.c:336-349`): true for code with an `-on_load` function,
    // false for code without one, and badarg for a reference and a term
    // that hold no prepared code.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang]).unwrap();
    let result = program
        .execute(
            "diagnostics_probe",
            "prepared",
            &[
                program.host_term(Term::binary(TARGET.to_vec())),
                program.host_term(Term::binary(ON_LOAD.to_vec())),
            ],
        )
        .unwrap();
    let expected = "{false,true,{error,badarg},{error,badarg}}";
    assert_eq!(text(&result.term, result.atoms()), expected);
    assert_eq!(
        reference(
            "diagnostics_probe:prepared(element(2, code:get_object_code(diagnostics_target)), element(2, code:get_object_code(diagnostics_on_load)))"
        ),
        format!("{expected}\n")
    );
}

#[test]
fn reads_a_list_of_process_info_items_of_the_caller_as_pinned_erts() {
    // `process_info_bif` reads the items of a list in their order, also a
    // repeated item, and `registered_name` gives `{registered_name, []}`
    // for a process without a name (`erl_bif_info.c:1555-1558`,
    // `1606-1632`). The crash report of `proc_lib` reads such a list of the
    // crashing process (`proc_lib.erl:968-971`).
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang, &lists]).unwrap();
    let result = program
        .execute("diagnostics_probe", "info_list", &[])
        .unwrap();
    let expected = "{[{registered_name,[]},{dictionary,[{probe_key,probe_value}]},{message_queue_len,0},{trap_exit,true},{status,running},{trap_exit,true}],[{registered_name,diagnostics_info_list}],[{heap_size,integer},{stack_size,integer},{reductions,integer}]}";
    assert_eq!(text(&result.term, result.atoms()), expected);
    assert_eq!(
        reference("diagnostics_probe:info_list()"),
        format!("{expected}\n")
    );
}

#[test]
fn charges_a_repeated_process_info_item_once_as_pinned_erts() {
    // `process_info_bif` reads each distinct item once into `part_res`
    // (`erl_bif_info.c:1134-1145`), and each read costs one reduction
    // (`erl_bif_info.c:1601`).
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let program = BeamProgram::load(&[PROBE, TARGET, &erlang, &lists]).unwrap();
    let result = program
        .execute("diagnostics_probe", "info_list_cost", &[])
        .unwrap();
    assert_eq!(text(&result.term, result.atoms()), "[2,2,5]");
    assert_eq!(reference("diagnostics_probe:info_list_cost()"), "[2,2,5]\n");
}
