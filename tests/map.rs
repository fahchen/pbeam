use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const MAP_PROBE: &[u8] = include_bytes!("../fixtures/map/map_probe.beam");
const MAP_ORDER_PROBE: &[u8] = include_bytes!("../fixtures/map/map_order_probe.beam");
const ATOM_MAP_PROBE: &[u8] = include_bytes!("../fixtures/map/atom_map_probe.beam");

/// Return the atom term of a name that the program holds.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

/// Return the list of the atoms in a comma-separated list of names.
fn atoms(program: &BeamProgram, names: &str) -> Term {
    Term::List(names.split(',').map(|name| atom(program, name)).collect())
}

fn map(program: &BeamProgram, pairs: &[(&str, Term)]) -> Term {
    let pairs = pairs
        .iter()
        .map(|(key, value)| (atom(program, key), value.clone()))
        .collect();
    Term::Map(Map::from_pairs(pairs).unwrap())
}

fn integers(keys: &[i64]) -> Term {
    Term::List(keys.iter().map(|key| Term::Integer(*key)).collect())
}

/// Run an expression under the pinned ERTS with the map fixtures in the code
/// path, and return its stdout.
fn erl_reference(expression: &str) -> Vec<u8> {
    let reference = Command::new("erl")
        .args([
            "-noshell",
            "-pa",
            "fixtures/map",
            "-eval",
            &format!("{expression}, halt()."),
        ])
        .output()
        .unwrap();
    assert!(reference.status.success());
    assert!(reference.stderr.is_empty());
    reference.stdout
}

#[test]
fn builds_matches_and_compares_maps_like_erts() {
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    assert_eq!(
        program.execute("map_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            atom("raw"),
            map(
                &program,
                &[
                    ("input", atom("cooked")),
                    ("ofd", atom("stdout")),
                    ("output", atom("raw"))
                ]
            ),
            atom("other"),
            atom("other"),
            atom("same"),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~p~n\", [map_probe:main()])"),
        b"{raw,#{input => cooked,output => raw,ofd => stdout},other,other,same}\n"
    );
}

#[test]
fn uses_a_flatmap_up_to_32_keys_and_a_hashmap_above_like_erts() {
    let hash33 = [
        18, 4, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17, 24, 14,
        33, 3, 16, 30, 21, 5, 27, 26, 22,
    ];
    let hash40 = [
        18, 4, 34, 12, 19, 29, 13, 2, 7, 38, 31, 8, 10, 23, 9, 15, 35, 36, 32, 1, 25, 28, 20, 6,
        11, 17, 24, 37, 40, 14, 33, 3, 16, 30, 21, 5, 27, 26, 22, 39,
    ];
    let program = BeamProgram::load(&[MAP_ORDER_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    assert_eq!(
        program.execute("map_order_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            integers(&(1..=32).collect::<Vec<_>>()),
            integers(&hash33),
            integers(&hash33),
            atom("same"),
            integers(&hash40),
            atom("same"),
            atom("different"),
            atom("v"),
            atom("missing"),
            Term::List(vec![Term::Integer(1), Term::Integer(2), atom("a")]),
            integers(&[-101714049538469044, -1, 5]),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_order_probe:main()])"),
        b"{[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          same,\
          [18,4,34,12,19,29,13,2,7,38,31,8,10,23,9,15,35,36,32,1,25,28,20,6,11,17,24,37,40,14,33,3,16,30,21,5,27,26,22,39],\
          same,different,v,missing,[1,2,a],[-101714049538469044,-1,5]}\n"
    );
}

#[test]
fn tests_map_keys_with_has_map_fields_like_erts() {
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    assert_eq!(
        program.execute("map_probe", "fields", &[]).unwrap(),
        atoms(&program, "both,none,none,none,found,missing,missing")
    );

    assert_eq!(
        erl_reference("io:format(\"~p~n\", [map_probe:fields()])"),
        b"[both,none,none,none,found,missing,missing]\n"
    );
}

#[test]
fn takes_the_badmap_path_when_add_gets_a_non_map() {
    // `add/1` tests is_map and then calls erlang:error({badmap, M}), which
    // PBeam does not resolve yet: `erlang:error/1` is not implemented.
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    assert!(
        program
            .execute("map_probe", "add", &[Term::Integer(1)])
            .unwrap_err()
            .to_string()
            .contains("unresolved external MFA erlang:error/1")
    );
}

#[test]
fn orders_and_hashes_atom_keys_by_the_erts_atom_index() {
    let program = BeamProgram::load(&[ATOM_MAP_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let atoms = |names: &str| atoms(&program, names);
    assert_eq!(
        program.execute("atom_map_probe", "main", &[]).unwrap(),
        Term::Tuple(vec![
            atom("pbeam_zeta"),
            // A hashmap of 40 static atoms: index order through the ERTS atom hash.
            atoms(
                "internal_error,utf16,total_active_tasks,error_handler,first,pending_driver,\
                 sigusr2,off_heap,complete,linked_in_driver,is_function,false,get_seq_token,\
                 yield,start_timer,all,start,dflag_unicode_io,more,bag,fun_info_mfa,\
                 http_response,get_definition,nodedown_reason,registered,link,port_call,add,\
                 dt_put_tag,reset,decentralized_counters,seq_trace,attributes,dotall,proc_sig,\
                 match_spec_compile,schedulers_online,dirty,memory_internal,call_error_handler"
            ),
            // A flatmap: true (1) and abs_path (26) are static; pbeam_zeta comes before
            // pbeam_alpha in AtU8.
            atoms("true,abs_path,pbeam_zeta,pbeam_alpha"),
            atom("pbeam_lit_b"),
            map(
                &program,
                &[
                    ("pbeam_lit_a", Term::Integer(1)),
                    ("pbeam_lit_b", Term::Integer(2))
                ]
            ),
            // LitT creates pbeam_lit_b first, because it is the first tuple element.
            atoms("pbeam_lit_b,pbeam_lit_a"),
            atom("pbeam_z_code"),
            // ERTS puts the AtU8 atoms before the LitT atoms: pbeam_z_code is in AtU8
            // and pbeam_a_lit is only in LitT, so index order is the reverse of name order.
            atoms("pbeam_z_code,pbeam_a_lit"),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [atom_map_probe:main()])"),
        b"{pbeam_zeta,[internal_error,utf16,total_active_tasks,error_handler,first,\
          pending_driver,sigusr2,off_heap,complete,linked_in_driver,is_function,false,\
          get_seq_token,yield,start_timer,all,start,dflag_unicode_io,more,bag,fun_info_mfa,\
          http_response,get_definition,nodedown_reason,registered,link,port_call,add,\
          dt_put_tag,reset,decentralized_counters,seq_trace,attributes,dotall,proc_sig,\
          match_spec_compile,schedulers_online,dirty,memory_internal,call_error_handler],\
          [true,abs_path,pbeam_zeta,pbeam_alpha],pbeam_lit_b,\
          #{pbeam_lit_b => 2,pbeam_lit_a => 1},[pbeam_lit_b,pbeam_lit_a],pbeam_z_code,\
          [pbeam_z_code,pbeam_a_lit]}\n"
    );
}

#[test]
fn orders_a_hashmap_of_all_static_atoms_like_erts() {
    let program = BeamProgram::load(&[ATOM_MAP_PROBE]).unwrap();
    let result = program
        .execute("atom_map_probe", "static_keys", &[])
        .unwrap();
    let Term::List(keys) = &result else {
        panic!("maps:keys/1 returns a list");
    };
    let names: Vec<&str> = keys
        .iter()
        .map(|key| match key {
            Term::Atom(atom) => atom.name(),
            other => panic!("static key {other:?} is not an atom"),
        })
        .collect();
    assert_eq!(names.len(), 1150);
    let expected = erl_reference(
        "io:format(\"~s~n\", \
         [lists:join(\",\", [atom_to_list(K) || K <- atom_map_probe:static_keys()])])",
    );
    // The ERTS 29.1.1 order starts with these keys in every run.
    assert!(expected.starts_with(
        b"exact_reductions,bin_is_7bit,pending_process,native,erl_debugger,internal_error,"
    ));
    assert_eq!(format!("{}\n", names.join(",")).into_bytes(), expected);
}
