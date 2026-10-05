use std::process::Command;

use pbeam_runtime::{BeamProgram, Map, Term};

const MAP_PROBE: &[u8] = include_bytes!("../fixtures/map/map_probe.beam");
const MAP_ORDER_PROBE: &[u8] = include_bytes!("../fixtures/map/map_order_probe.beam");
const ATOM_MAP_PROBE: &[u8] = include_bytes!("../fixtures/map/atom_map_probe.beam");
const MAP_KEY_PROBE: &[u8] = include_bytes!("../fixtures/map/map_key_probe.beam");

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

/// Return the list of the bytes of a string, as Erlang writes `"..."`.
fn string(text: &str) -> Term {
    Term::List(
        text.bytes()
            .map(|byte| Term::Integer(byte.into()))
            .collect(),
    )
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

#[test]
fn orders_and_hashes_list_keys_like_erts() {
    let program = BeamProgram::load(&[MAP_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let list = Term::List;
    let int = Term::Integer;
    // A flatmap: the atom a first, then [], then lists in the term order. The atom a in
    // [a] compares by name, so [a] comes after the strings "a" and "b" and after [300].
    let flat = |big: Option<Term>| {
        let mut keys = vec![
            atom("a"),
            list(vec![]),
            list(vec![int(1), integers(&[2])]),
            string("a"),
            string("b"),
            integers(&[300]),
        ];
        keys.extend(big);
        keys.push(list(vec![atom("a")]));
        list(keys)
    };
    let s = string;
    let hash = vec![
        s("\x1b[A"),
        s("k16"),
        list(vec![integers(&[1]), atom("true")]),
        s("k23"),
        s("k17"),
        s("k13"),
        s("k27"),
        s("k08"),
        s("k30"),
        s("k21"),
        s("k20"),
        s("k10"),
        s("k01"),
        s("k05"),
        s("k03"),
        s("k18"),
        s("k07"),
        atom("default"),
        s("k29"),
        s("k28"),
        s("k22"),
        s("k11"),
        s("k02"),
        s("k26"),
        integers(&[300, 1]),
        s("k14"),
        s("k25"),
        s("k12"),
        s("k19"),
        s("k09"),
        s("k15"),
        s("k04"),
        s("k06"),
        s("k24"),
    ];
    assert_eq!(
        program.execute("map_key_probe", "list_keys", &[]).unwrap(),
        list(vec![
            flat(None),
            int(3),
            list(hash),
            int(31),
            int(3),
            int(7),
            int(33),
            int(31),
            flat(Some(integers(&[300, 1]))),
            int(9),
            int(34),
            int(35),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_key_probe:list_keys()])"),
        b"[[a,[],[1,[2]],[97],[98],[300],[a]],3,\
          [[27,91,65],[107,49,54],[[1],true],[107,50,51],[107,49,55],[107,49,51],[107,50,55],\
          [107,48,56],[107,51,48],[107,50,49],[107,50,48],[107,49,48],[107,48,49],[107,48,53],\
          [107,48,51],[107,49,56],[107,48,55],default,[107,50,57],[107,50,56],[107,50,50],\
          [107,49,49],[107,48,50],[107,50,54],[300,1],[107,49,52],[107,50,53],[107,49,50],\
          [107,49,57],[107,48,57],[107,49,53],[107,48,52],[107,48,54],[107,50,52]],31,\
          3,7,33,31,[a,[],[1,[2]],[97],[98],[300],[300,1],[a]],9,34,35]\n"
    );
}

#[test]
fn hashes_nested_lists_and_string_chunks_like_erts() {
    let program = BeamProgram::load(&[MAP_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let list = Term::List;
    let int = Term::Integer;
    let (min, max) = (-(1i64 << 59), (1i64 << 59) - 1);
    assert_eq!(
        program.execute("map_key_probe", "hash_keys", &[]).unwrap(),
        list(vec![
            int(18),
            int(4),
            integers(&[1, 2, 3, 4, 5, 6, 7, 8, 9]),
            integers(&[min]),
            int(12),
            list(vec![integers(&[1]), integers(&[2])]),
            int(19),
            int(13),
            int(2),
            int(7),
            integers(&[256, 1, 2]),
            int(8),
            int(10),
            int(23),
            int(9),
            integers(&[1, 2, 3, 4, 5]),
            list(vec![list(vec![])]),
            int(15),
            int(1),
            int(20),
            int(6),
            list(vec![]),
            int(11),
            list(vec![int(1), atom("true"), int(2), int(3)]),
            int(17),
            int(24),
            list(vec![atom("true"), list(vec![atom("false")]), atom("true")]),
            int(14),
            int(3),
            int(16),
            list(vec![list(vec![list(vec![])])]),
            int(21),
            int(5),
            int(0),
            int(22),
            integers(&[max]),
            integers(&[-1]),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_key_probe:hash_keys()])"),
        b"[18,4,[1,2,3,4,5,6,7,8,9],[-576460752303423488],12,[[1],[2]],19,13,2,7,[256,1,2],8,\
          10,23,9,[1,2,3,4,5],[[]],15,1,20,6,[],11,[1,true,2,3],17,24,[true,[false],true],14,3,\
          16,[[[]]],21,5,0,22,[576460752303423487],[-1]]\n"
    );
}

#[test]
fn keeps_keys_with_equal_hashes_in_a_collision_node_like_erts() {
    let program = BeamProgram::load(&[MAP_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let (first, second) = (
        integers(&[246, 157, 203, 77, 19, 254, 19, 93]),
        integers(&[161, 92, 65, 50, 231, 24, 153, 137]),
    );
    // The two keys have equal hashes, so ERTS keeps them next to each other in the term order.
    let keys = Term::List(
        [18, 4, 12, 19, 29, 13, 2]
            .map(Term::Integer)
            .into_iter()
            .chain([second.clone(), first.clone()])
            .chain(
                [
                    7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17, 24, 14, 3, 16, 30, 21,
                    5, 27, 26, 22,
                ]
                .map(Term::Integer),
            )
            .collect(),
    );
    assert_eq!(
        program
            .execute("map_key_probe", "collision_keys", &[])
            .unwrap(),
        Term::List(vec![
            keys,
            atom("a"),
            atom("b"),
            atom("b"),
            atom("true"),
            atom("c"),
            atom("d"),
            Term::List(vec![second, first]),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_key_probe:collision_keys()])"),
        b"[[18,4,12,19,29,13,2,[161,92,65,50,231,24,153,137],[246,157,203,77,19,254,19,93],7,31,\
          8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,3,16,30,21,5,27,26,22],a,b,b,true,c,d,\
          [[161,92,65,50,231,24,153,137],[246,157,203,77,19,254,19,93]]]\n"
    );
}
