//! Runs map construction, matching, comparison and the flatmap and hashmap limits on PBeam and
//! compares them with the pinned ERTS.

use std::process::Command;

use pbeam_runtime::{Atom, AtomTable, BeamProgram, Map, Term};

#[path = "../common/mod.rs"]
mod common;

use common::pinned_beam;

const MAP_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_probe.beam");
const MAP_ORDER_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_order_probe.beam");
const ATOM_MAP_PROBE: &[u8] = include_bytes!("../../fixtures/map/atom_map_probe.beam");
const MAP_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_key_probe.beam");
const BINARY_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/binary_key_probe.beam");
const MAP_SIZE_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_size_probe.beam");
const TUPLE_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/tuple_key_probe.beam");
const MAP_SHARE_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_share_probe.beam");
const MAP_ELEMENTS_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_elements_probe.beam");
const REFERENCE_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/reference_key_probe.beam");
const PID_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/pid_key_probe.beam");

/// The numbers `N` of the binary keys `<<"kNN">>`, 1 to 33, in the order of a hashmap of these 33
/// keys under the pinned ERTS.
const BINARY_HASH_ORDER: [i64; 33] = [
    17, 26, 23, 5, 31, 28, 20, 29, 12, 25, 21, 3, 11, 7, 15, 16, 30, 1, 6, 9, 14, 10, 22, 27, 13,
    18, 32, 2, 24, 33, 19, 8, 4,
];

/// The `{Row, Col}` keys of a hashmap of the 36 cells `{0..5, 0..5}` in the order of the pinned
/// ERTS.
const GRID_HASH_ORDER: [(i64, i64); 36] = [
    (0, 4),
    (1, 2),
    (2, 1),
    (0, 3),
    (4, 3),
    (0, 0),
    (5, 2),
    (0, 2),
    (2, 0),
    (1, 4),
    (3, 2),
    (3, 0),
    (2, 4),
    (4, 1),
    (2, 3),
    (3, 5),
    (4, 2),
    (4, 0),
    (5, 5),
    (5, 1),
    (0, 1),
    (2, 2),
    (1, 0),
    (4, 4),
    (2, 5),
    (3, 1),
    (3, 3),
    (0, 5),
    (5, 3),
    (3, 4),
    (4, 5),
    (5, 4),
    (5, 0),
    (1, 5),
    (1, 1),
    (1, 3),
];

/// The `{Row, Col}` keys of a hashmap of the 32 cells `{0..3, 0..7}` and the key `{9, 9}` in the
/// order of the pinned ERTS.
const GROWN_HASH_ORDER: [(i64, i64); 33] = [
    (0, 4),
    (1, 2),
    (2, 1),
    (2, 6),
    (0, 3),
    (0, 0),
    (0, 2),
    (2, 0),
    (1, 4),
    (3, 2),
    (2, 7),
    (1, 7),
    (3, 0),
    (2, 4),
    (0, 6),
    (2, 3),
    (3, 6),
    (3, 5),
    (0, 7),
    (0, 1),
    (2, 2),
    (1, 0),
    (2, 5),
    (3, 1),
    (1, 6),
    (3, 7),
    (3, 3),
    (0, 5),
    (3, 4),
    (1, 5),
    (1, 1),
    (9, 9),
    (1, 3),
];

/// Return the atom term of a name that the program holds.
fn atom(program: &BeamProgram, name: &str) -> Term {
    Term::Atom(program.atom(name).unwrap())
}

/// Return the list of the atoms in a comma-separated list of names.
fn atoms(program: &BeamProgram, names: &str) -> Term {
    Term::list_from(names.split(',').map(|name| atom(program, name)))
}

fn map(program: &BeamProgram, pairs: &[(&str, Term)]) -> Term {
    let pairs = pairs
        .iter()
        .map(|(key, value)| (atom(program, key), value.clone()))
        .collect();
    Term::Map(Map::from_pairs(pairs, program.atoms()).unwrap())
}

fn integers(keys: &[i64]) -> Term {
    Term::list_from(keys.iter().map(|key| Term::Integer(*key)))
}

/// Return the list of the bytes of a string, as Erlang writes `"..."`.
fn string(text: &str) -> Term {
    Term::list_from(text.bytes().map(|byte| Term::Integer(byte.into())))
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
    assert_eq!(reference.stderr, [] as [u8; 0]);
    reference.stdout
}

#[test]
fn builds_matches_and_compares_maps_like_erts() {
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    assert_eq!(
        program.execute("map_probe", "main", &[]).unwrap().term,
        Term::tuple(vec![
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
        program
            .execute("map_order_probe", "main", &[])
            .unwrap()
            .term,
        Term::tuple(vec![
            integers(&(1..=32).collect::<Vec<_>>()),
            integers(&hash33),
            integers(&hash33),
            atom("same"),
            integers(&hash40),
            atom("same"),
            atom("different"),
            atom("v"),
            atom("missing"),
            Term::list_from(vec![Term::Integer(1), Term::Integer(2), atom("a")]),
            integers(&[-101_714_049_538_469_044, -1, 5]),
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
        program.execute("map_probe", "fields", &[]).unwrap().term,
        atoms(&program, "both,none,none,none,found,missing,missing")
    );

    assert_eq!(
        erl_reference("io:format(\"~p~n\", [map_probe:fields()])"),
        b"[both,none,none,none,found,missing,missing]\n"
    );
}

#[test]
fn gets_map_elements_into_the_source_register_like_erts() {
    // `pick/1` gets four keys of the map in `x0` into `x0` to `x3`, and
    // `same/1` keeps the map in `x0` for its fail path.
    let program = BeamProgram::load(&[MAP_ELEMENTS_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let found = |path: &str| Term::tuple(vec![atom("t"), atom(path), atom("one"), atom("nil")]);
    assert_eq!(
        program
            .execute("map_elements_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            found("p"),
            found("q"),
            atom("missing"),
            atom("missing"),
            atom("missing"),
            found("p"),
            atom("missing"),
            Term::tuple(vec![atom("t"), atom("p"), Term::Integer(4)]),
            Term::tuple(vec![atom("missing"), Term::Integer(1)]),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_elements_probe:main()])"),
        b"[{t,p,one,nil},{t,q,one,nil},missing,missing,missing,{t,p,one,nil},missing,{t,p,4},{missing,1}]\n"
    );
}

#[test]
fn takes_the_badmap_path_when_add_gets_a_non_map() {
    // `add/1` tests is_map and then calls erlang:error({badmap, M}), which
    // raises class `error` with the reason `{badmap, 1}`. The stack trace of
    // `erlang:error/1` is the frame of its caller with its arity
    // (`beam_common.c:1011-1027`, `utils.c:3430-3455`), with the file and
    // the line of the call. The root process has no caller frames; on ERTS,
    // the frames of the `erl -eval` caller follow.
    let program = BeamProgram::load(&[MAP_PROBE]).unwrap();
    let error = program
        .execute("map_probe", "add", &[program.host_term(Term::Integer(1))])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains(
            "root process exited with {{badmap,1},[{map_probe,add,1,[{file,\"fixtures/map/map_probe.erl\"},{line,7}]}]}"
        ),
        "{error}"
    );

    assert_eq!(
        erl_reference(
            "io:format(\"~p~n\", [try map_probe:add(1) catch Class:Reason -> {Class, Reason} end])"
        ),
        b"{error,{badmap,1}}\n"
    );
}

#[test]
fn orders_and_hashes_atom_keys_by_the_erts_atom_index() {
    let program = BeamProgram::load(&[ATOM_MAP_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let atoms = |names: &str| atoms(&program, names);
    assert_eq!(
        program.execute("atom_map_probe", "main", &[]).unwrap().term,
        Term::tuple(vec![
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
    let Some(keys) = result
        .term
        .proper_list()
        .map(std::iter::Iterator::collect::<Vec<_>>)
    else {
        panic!("maps:keys/1 returns a list");
    };
    let names: Vec<&str> = keys
        .iter()
        .map(|key| match key {
            Term::Atom(atom) => result.atoms().name_of(atom),
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
    let list = |elements: Vec<Term>| Term::list_from(elements);
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
        program
            .execute("map_key_probe", "list_keys", &[])
            .unwrap()
            .term,
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
    let list = |elements: Vec<Term>| Term::list_from(elements);
    let int = Term::Integer;
    let (min, max) = (-(1i64 << 59), (1i64 << 59) - 1);
    assert_eq!(
        program
            .execute("map_key_probe", "hash_keys", &[])
            .unwrap()
            .term,
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
    let keys = Term::list_from(
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
            ),
    );
    assert_eq!(
        program
            .execute("map_key_probe", "collision_keys", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            keys,
            atom("a"),
            atom("b"),
            atom("b"),
            atom("true"),
            atom("c"),
            atom("d"),
            Term::list_from(vec![second, first]),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_key_probe:collision_keys()])"),
        b"[[18,4,12,19,29,13,2,[161,92,65,50,231,24,153,137],[246,157,203,77,19,254,19,93],7,31,\
          8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,3,16,30,21,5,27,26,22],a,b,b,true,c,d,\
          [[161,92,65,50,231,24,153,137],[246,157,203,77,19,254,19,93]]]\n"
    );
}

#[test]
fn loads_map_literals_with_binary_keys_like_erts() {
    let program = BeamProgram::load(&[BINARY_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let int = Term::Integer;
    let binary = |text: &str| Term::binary(text.as_bytes().to_vec());
    // A flatmap orders binary keys in the term order, after the numbers, atoms, and lists. A
    // hashmap orders them by the ERTS internal hash of each binary.
    let hash_keys = BINARY_HASH_ORDER.map(|key| binary(&format!("k{key:02}")));
    assert_eq!(
        program
            .execute("binary_key_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::list_from(vec![binary("a"), binary("b")]),
            Term::list_from(vec![
                int(1),
                atom("a"),
                string("s"),
                binary(""),
                binary("a"),
                binary("ab"),
                binary("b"),
            ]),
            Term::list_from(hash_keys.to_vec()),
            int(0),
            int(0),
            int(7),
            int(17),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [binary_key_probe:main()])"),
        b"[[<<97>>,<<98>>],[1,a,[115],<<>>,<<97>>,<<97,98>>,<<98>>],[<<107,49,55>>,<<107,50,54>>,\
          <<107,50,51>>,<<107,48,53>>,<<107,51,49>>,<<107,50,56>>,<<107,50,48>>,<<107,50,57>>,\
          <<107,49,50>>,<<107,50,53>>,<<107,50,49>>,<<107,48,51>>,<<107,49,49>>,<<107,48,55>>,\
          <<107,49,53>>,<<107,49,54>>,<<107,51,48>>,<<107,48,49>>,<<107,48,54>>,<<107,48,57>>,\
          <<107,49,52>>,<<107,49,48>>,<<107,50,50>>,<<107,50,55>>,<<107,49,51>>,<<107,49,56>>,\
          <<107,51,50>>,<<107,48,50>>,<<107,50,52>>,<<107,51,51>>,<<107,49,57>>,<<107,48,56>>,\
          <<107,48,52>>],0,0,7,17]\n"
    );
}

#[test]
fn puts_binary_keys_into_maps_like_erts() {
    let maps = pinned_beam("maps");
    let program = BeamProgram::load(&[&maps, BINARY_KEY_PROBE]).unwrap();
    let int = Term::Integer;
    let binary = |text: &str| Term::binary(text.as_bytes().to_vec());
    let pairs = |pairs: &[(&str, i64)]| {
        Term::list_from(
            pairs
                .iter()
                .map(|(key, value)| Term::tuple(vec![binary(key), int(*value)])),
        )
    };
    // `maps:put/3` puts `<<"ab">>` between `<<"a">>` and `<<"b">>` of a flatmap, and the 33rd key
    // changes a flatmap to a hashmap in the hash order.
    let grown = Term::list_from(
        BINARY_HASH_ORDER
            .map(|key| Term::tuple(vec![binary(&format!("k{key:02}")), int(key)]))
            .to_vec(),
    );
    assert_eq!(
        program
            .execute("binary_key_probe", "update", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            pairs(&[("a", 0), ("ab", 1), ("b", 0)]),
            pairs(&[("a", 2), ("ab", 1), ("b", 0)]),
            grown,
            int(1),
            int(33),
        ])
    );

    let grown = BINARY_HASH_ORDER
        .map(|key| format!("{{<<107,{},{}>>,{key}}}", 48 + key / 10, 48 + key % 10))
        .join(",");
    assert_eq!(
        String::from_utf8(erl_reference(
            "io:format(\"~w~n\", [binary_key_probe:update()])"
        ))
        .unwrap(),
        format!(
            "[[{{<<97>>,0}},{{<<97,98>>,1}},{{<<98>>,0}}],[{{<<97>>,2}},{{<<97,98>>,1}},{{<<98>>,0}}],\
             [{grown}],1,33]\n"
        )
    );
}

#[test]
fn gives_the_map_size_like_erts() {
    // The pinned compiler emits each `map_size/1` call of the fixture as `gc_bif`: with the fail
    // label 0 in `main/0` and `body/1`, and with a fail label in the guard of `guard/1`.
    let program = BeamProgram::load(&[MAP_SIZE_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let int = Term::Integer;
    let badmap =
        |term: Term| Term::tuple(vec![atom("error"), Term::tuple(vec![atom("badmap"), term])]);
    assert_eq!(
        program.execute("map_size_probe", "main", &[]).unwrap().term,
        Term::list_from(vec![
            int(2),
            int(2),
            int(0),
            int(33),
            int(34),
            atom("small"),
            atom("big"),
            atom("small"),
            atom("other"),
            atom("other"),
            badmap(atom("foo")),
            badmap(Term::list_from(vec![int(1)])),
            badmap(Term::binary(b"ab".to_vec())),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_size_probe:main()])"),
        b"[2,2,0,33,34,small,big,small,other,other,{error,{badmap,foo}},{error,{badmap,[1]}},\
          {error,{badmap,<<97,98>>}}]\n"
    );
}

#[test]
fn loads_and_updates_maps_with_tuple_keys_like_erts() {
    let maps = pinned_beam("maps");
    let program = BeamProgram::load(&[&maps, TUPLE_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let int = Term::Integer;
    let tuple = Term::tuple;
    let pair = |key: Term, value: &str| tuple(vec![key, atom(value)]);
    // A flatmap orders two top-level atoms by atom index (value before abs) and each other two
    // keys in the term order: a tuple by its size, then by its elements, with atoms by name.
    let keys = [
        int(3),
        atom("value"),
        atom("abs"),
        tuple(vec![]),
        tuple(vec![int(1)]),
        tuple(vec![atom("abs")]),
        tuple(vec![atom("value")]),
        tuple(vec![int(0), int(5)]),
        tuple(vec![int(1), int(2)]),
        tuple(vec![Term::list_from(vec![int(1)]), tuple(vec![int(2)])]),
        Term::list_from(vec![int(1)]),
    ];
    let mut exact: Vec<Term> = ["h", "f", "g", "j", "c", "e", "d", "n", "m", "k", "i"]
        .iter()
        .zip(keys.iter())
        .map(|(value, key)| pair(key.clone(), value))
        .collect();
    exact.insert(9, pair(tuple(vec![int(2), int(1)]), "l"));
    assert_eq!(
        program
            .execute("tuple_key_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::list_from(keys.to_vec()),
            atom("a"),
            atom("d"),
            atom("c"),
            atom("j"),
            Term::list_from(exact),
            atom("true"),
            atom("error"),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [tuple_key_probe:main()])"),
        b"[[3,value,abs,{},{1},{abs},{value},{0,5},{1,2},{[1],{2}},[1]],a,d,c,j,[{3,h},{value,f},\
          {abs,g},{{},j},{{1},c},{{abs},e},{{value},d},{{0,5},n},{{1,2},m},{{2,1},l},\
          {{[1],{2}},k},{[1],i}],true,error]\n"
    );
}

#[test]
fn puts_tuple_keys_with_maps_put_like_erts() {
    let program = BeamProgram::load(&[TUPLE_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let int = Term::Integer;
    let cell = |(row, col): (i64, i64)| Term::tuple(vec![int(row), int(col)]);
    let cells = |cells: &[(i64, i64)]| Term::list_from(cells.iter().copied().map(cell));
    // `maps:put/3` keeps a flatmap of 32 cells in the term order. The 33rd key, from `maps:put/3`
    // or from the 33rd cell, changes the flatmap to a hashmap in the ERTS hash order.
    let small: Vec<(i64, i64)> = (0..4)
        .flat_map(|row| (0..8).map(move |col| (row, col)))
        .collect();
    assert_eq!(
        program
            .execute("tuple_key_probe", "grid", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            cells(&small),
            cells(&GRID_HASH_ORDER),
            cells(&GROWN_HASH_ORDER),
            atom("replaced"),
            int(55),
            int(36),
            int(55),
            Term::tuple(vec![atom("ok"), atom("grown")]),
        ])
    );

    let text = |cells: &[(i64, i64)]| {
        let cells: Vec<String> = cells
            .iter()
            .map(|(row, col)| format!("{{{row},{col}}}"))
            .collect();
        cells.join(",")
    };
    assert_eq!(
        String::from_utf8(erl_reference(
            "io:format(\"~w~n\", [tuple_key_probe:grid()])"
        ))
        .unwrap(),
        format!(
            "[[{}],[{}],[{}],replaced,55,36,55,{{ok,grown}}]\n",
            text(&small),
            text(&GRID_HASH_ORDER),
            text(&GROWN_HASH_ORDER)
        )
    );
}

#[test]
fn raises_badmap_and_badkey_for_tuple_keys_like_erts() {
    let program = BeamProgram::load(&[TUPLE_KEY_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let error = |tag: &str, term: Term| {
        Term::tuple(vec![atom("error"), Term::tuple(vec![atom(tag), term])])
    };
    let missing = || Term::tuple(vec![Term::Integer(9), Term::Integer(9)]);
    assert_eq!(
        program
            .execute("tuple_key_probe", "errors", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            error("badmap", atom("not_a_map")),
            error("badkey", missing()),
            error("badmap", atom("not_a_map")),
            error("badkey", missing()),
            error("badkey", missing()),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [tuple_key_probe:errors()])"),
        b"[{error,{badmap,not_a_map}},{error,{badkey,{9,9}}},{error,{badmap,not_a_map}},\
          {error,{badkey,{9,9}}},{error,{badkey,{9,9}}}]\n"
    );
}

#[test]
fn keeps_each_copy_of_a_map_when_another_copy_changes_like_erts() {
    // `main/0` changes copies of one map in a register, a stack slot, a fun, and a message with
    // `put_map_exact`, `put_map_assoc`, `maps:merge/2`, `maps:remove/2`, and `maps:take/2`. The
    // keys are integers, so the order of the ERTS output does not depend on atom indexes.
    let program = BeamProgram::load(&[MAP_SHARE_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let map = |pairs: &[(i64, &str)]| {
        let pairs = pairs
            .iter()
            .map(|(key, value)| (Term::Integer(*key), atom(value)))
            .collect();
        Term::Map(Map::from_pairs(pairs, program.atoms()).unwrap())
    };
    assert_eq!(
        program
            .execute("map_share_probe", "main", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            map(&[(1, "a"), (2, "b")]),
            map(&[(1, "a"), (2, "b")]),
            map(&[(1, "c"), (2, "b")]),
            map(&[(1, "a"), (2, "b"), (3, "d")]),
            map(&[(1, "a"), (2, "e")]),
            map(&[(1, "a"), (2, "f"), (4, "g")]),
            map(&[(2, "b")]),
            Term::tuple(vec![atom("b"), map(&[(1, "a")])]),
            map(&[(1, "h"), (2, "b")]),
            map(&[(1, "a"), (2, "b")]),
            map(&[(1, "a"), (2, "i")]),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_share_probe:main()])"),
        b"[#{1 => a,2 => b},#{1 => a,2 => b},#{1 => c,2 => b},#{1 => a,2 => b,3 => d},\
          #{1 => a,2 => e},#{1 => a,2 => f,4 => g},#{2 => b},{b,#{1 => a}},#{1 => h,2 => b},\
          #{1 => a,2 => b},#{1 => a,2 => i}]\n"
    );
}

#[test]
fn keeps_a_shared_flatmap_when_a_copy_becomes_a_hashmap_like_erts() {
    // The 33 keys of the hashmap are in the ERTS hash order of `uses_a_flatmap_up_to_32_keys_and_a_hashmap_above_like_erts`.
    let hash33 = [
        18, 4, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17, 24, 14,
        33, 3, 16, 30, 21, 5, 27, 26, 22,
    ];
    let program = BeamProgram::load(&[MAP_SHARE_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let int = Term::Integer;
    assert_eq!(
        program
            .execute("map_share_probe", "grow", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            integers(&(1..=32).collect::<Vec<_>>()),
            integers(&hash33),
            int(32),
            int(33),
            int(1),
            int(1),
            atom("one"),
            atom("uno"),
            atom("true"),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_share_probe:grow()])"),
        b"[[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32],\
          [18,4,12,19,29,13,2,7,31,8,10,23,9,15,32,1,25,28,20,6,11,17,24,14,33,3,16,30,21,5,27,26,22],\
          32,33,1,1,one,uno,true]\n"
    );
}

#[test]
fn raises_badkey_and_keeps_the_shared_map_like_erts() {
    let program = BeamProgram::load(&[MAP_SHARE_PROBE]).unwrap();
    let atom = |name: &str| atom(&program, name);
    let int = Term::Integer;
    let badkey =
        |key: Term| Term::tuple(vec![atom("error"), Term::tuple(vec![atom("badkey"), key])]);
    assert_eq!(
        program
            .execute("map_share_probe", "errors", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            badkey(int(2)),
            badkey(int(34)),
            Term::Map(Map::from_pairs(vec![(int(1), atom("a"))], program.atoms()).unwrap(),),
            int(33),
            int(1),
        ])
    );

    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_share_probe:errors()])"),
        b"[{error,{badkey,2}},{error,{badkey,34}},#{1 => a},33,1]\n"
    );
}

const MAP_TAKE_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_take_probe.beam");

/// Check `function/0` of the map-take probe against the pinned ERTS 29.1.1
/// result. `erts` is the text of that run and `expected` is the same value
/// as a term. Both runtimes are compared with `expected`, so a wrong
/// expected value fails: the pinned text is a literal of this test, and
/// PBeam runs the compiled fixture. The keys are integers, so the map order
/// does not depend on the atom table.
fn take_probe(function: &str, expected: Term, erts: &[u8]) {
    let program = BeamProgram::load(&[MAP_TAKE_PROBE]).unwrap();
    let result = program.execute("map_take_probe", function, &[]).unwrap();
    assert_eq!(
        erl_reference(&format!(
            "io:format(\"~w~n\", [map_take_probe:{function}()])"
        )),
        erts,
        "{function} must keep the pinned ERTS reference run"
    );
    assert_eq!(
        result.term, expected,
        "{function} must give the pinned ERTS result through PBeam"
    );
}

/// Return a map term of the given integer keys and values.
fn int_map(pairs: &[(i64, Term)]) -> Term {
    Term::Map(
        Map::from_pairs(
            pairs
                .iter()
                .map(|(key, value)| (Term::Integer(*key), value.clone()))
                .collect(),
            &AtomTable::new(),
        )
        .unwrap(),
    )
}

/// Return the static atom term of a name.
fn static_atom(name: &str) -> Term {
    Term::Atom(Atom::from_static(name).expect("a static atom name"))
}

#[test]
fn keeps_a_same_register_update_with_its_aliases_like_erts() {
    // The source and the destination of `put_map_exact` and of
    // `put_map_assoc` are one register (`exact/2` and `assoc/3` of the
    // fixture). The take path changes the map in place. The key and the
    // value of the pair of `assoc/3` are registers of the caller, so every
    // operand read completes before the change. Every old root and each
    // alias keeps the pinned ERTS value.
    let source = int_map(&[(1, Term::Integer(1)), (2, Term::Integer(2))]);
    let alias = int_map(&[
        (1, Term::Integer(1)),
        (2, Term::Integer(2)),
        (7, Term::Integer(7)),
    ]);
    take_probe(
        "aliases",
        Term::list_from(vec![
            int_map(&[(1, Term::Integer(9)), (2, Term::Integer(2))]),
            source,
            Term::Integer(7),
            Term::Integer(9),
            alias,
            int_map(&[
                (1, Term::Integer(1)),
                (2, Term::Integer(2)),
                (7, Term::Integer(9)),
            ]),
            int_map(&[(1, Term::Integer(3)), (2, Term::Integer(2))]),
        ]),
        b"[#{1 => 9,2 => 2},#{1 => 1,2 => 2},7,9,#{1 => 1,2 => 2,7 => 7},\
          #{1 => 1,2 => 2,7 => 9},#{1 => 3,2 => 2}]\n",
    );
}

#[test]
fn keeps_a_value_that_aliases_the_map_like_erts() {
    // `exact_self/1`, `assoc_self/2`, and `exact_self_key/2` name the source
    // map in the value operand, so the value operand is the register that the
    // change also writes (`{put_map_exact,{f,0},{x,0},{x,0},1,
    // {list,[{integer,1},{x,0}]}}` and the two other forms). ERTS reads the
    // value before the change, so the new map holds the old map.
    // `shared_value/1` puts a map that the source holds in the value
    // (`{list,[{integer,1},{x,1}]}`).
    let program = BeamProgram::load(&[MAP_TAKE_PROBE]).unwrap();
    let i = Term::Atom(program.atom("i").unwrap());
    let x = static_atom("x");
    let source = int_map(&[(1, Term::Integer(0)), (2, Term::Integer(2))]);
    let two = int_map(&[(2, Term::Integer(2))]);
    let inner = Term::Map(Map::from_pairs(vec![(x, Term::Integer(1))], program.atoms()).unwrap());
    take_probe(
        "aliasing",
        Term::list_from(vec![
            int_map(&[(1, source.clone()), (2, Term::Integer(2))]),
            int_map(&[(2, Term::Integer(2)), (7, two)]),
            int_map(&[(1, source), (2, Term::Integer(2))]),
            Term::Map(
                Map::from_pairs(
                    vec![
                        (Term::Integer(1), inner.clone()),
                        (Term::Integer(2), Term::Integer(2)),
                        (i, inner),
                    ],
                    program.atoms(),
                )
                .unwrap(),
            ),
        ]),
        b"[#{1 => #{1 => 0,2 => 2},2 => 2},#{2 => 2,7 => #{2 => 2}},\
          #{1 => #{1 => 0,2 => 2},2 => 2},#{1 => #{x => 1},2 => 2,i => #{x => 1}}]\n",
    );
}

#[test]
fn keeps_a_shared_map_and_its_descendants_like_erts() {
    // An update of a map that another owner holds copies the form and the
    // nodes on the path of the key, and each old root keeps its pairs. The
    // shared inner term is not written.
    let inner = int_map(&[(1, Term::Integer(1))]);
    let old = int_map(&[(1, inner.clone())]);
    take_probe(
        "retained",
        Term::list_from(vec![
            old.clone(),
            old,
            int_map(&[(1, inner.clone()), (2, Term::Integer(2))]),
            inner.clone(),
            int_map(&[(1, inner.clone())]),
            int_map(&[(1, int_map(&[(1, Term::Integer(1)), (2, Term::Integer(2))]))]),
            inner,
            int_map(&[(1, Term::Integer(1)), (2, Term::Integer(2))]),
        ]),
        b"[#{1 => #{1 => 1}},#{1 => #{1 => 1}},#{1 => #{1 => 1},2 => 2},#{1 => 1},\
          #{1 => #{1 => 1}},#{1 => #{1 => 1,2 => 2}},#{1 => 1},#{1 => 1,2 => 2}]\n",
    );
}

#[test]
fn raises_badkey_and_badmap_for_a_same_register_update_like_erts() {
    // An invalid key and a term that is not a map keep the pinned ERTS
    // exception class and reason: `error:{badkey, 2}` and
    // `error:{badmap, not_a_map}`.
    let program = BeamProgram::load(&[MAP_TAKE_PROBE]).unwrap();
    let not_a_map = Term::Atom(program.atom("not_a_map").unwrap());
    take_probe(
        "keys",
        Term::list_from(vec![
            Term::tuple(vec![static_atom("badkey"), Term::Integer(2)]),
            int_map(&[(1, Term::Integer(2))]),
            int_map(&[(1, Term::Integer(1)), (2, Term::Integer(2))]),
            int_map(&[(1, Term::Integer(3))]),
            int_map(&[(1, Term::Integer(1))]),
            Term::tuple(vec![static_atom("badmap"), not_a_map]),
        ]),
        b"[{badkey,2},#{1 => 2},#{1 => 1,2 => 2},#{1 => 3},#{1 => 1},{badmap,not_a_map}]\n",
    );
}

#[test]
fn jumps_to_a_fail_label_in_a_guard_like_erts() {
    // `put_map_exact` of a guard has a fail label that is not 0. A missing
    // key fails the guard, and the source register keeps its value.
    let program = BeamProgram::load(&[MAP_TAKE_PROBE]).unwrap();
    let present = Term::Atom(program.atom("present").unwrap());
    let absent = Term::Atom(program.atom("absent").unwrap());
    take_probe(
        "guarded",
        Term::list_from(vec![present, absent, int_map(&[(1, Term::Integer(1))])]),
        b"[present,absent,#{1 => 1}]\n",
    );
}

#[test]
fn keeps_the_source_of_a_caught_exception_like_erts() {
    // A caught exception keeps the source register and each alias. The
    // catch gives the class `error` and the reason `{badkey, 2}`.
    let source = int_map(&[(1, Term::Integer(1))]);
    take_probe(
        "caught",
        Term::list_from(vec![
            Term::tuple(vec![
                static_atom("error"),
                Term::tuple(vec![static_atom("badkey"), Term::Integer(2)]),
                source.clone(),
                source.clone(),
            ]),
            source.clone(),
            source,
        ]),
        b"[{error,{badkey,2},#{1 => 1},#{1 => 1}},#{1 => 1},#{1 => 1}]\n",
    );
}

#[test]
fn raises_badmap_for_a_same_register_update_of_a_non_map_like_erts() {
    // `badmap/1` holds a term that is not a map in the source register and
    // calls `put_map_exact` on it with a catch tag, so the process runs its
    // handler after the raise.
    let program = BeamProgram::load(&[MAP_TAKE_PROBE]).unwrap();
    let not_a_map = Term::Atom(program.atom("not_a_map").unwrap());
    let argument = program.host_term(not_a_map.clone());
    assert_eq!(
        program
            .execute("map_take_probe", "badmap", std::slice::from_ref(&argument))
            .unwrap()
            .term,
        Term::tuple(vec![static_atom("badmap"), not_a_map])
    );
    assert_eq!(
        erl_reference("io:format(\"~w~n\", [map_take_probe:badmap(not_a_map)])"),
        b"{badmap,not_a_map}\n"
    );
}

#[test]
fn runs_map_operations_with_reference_keys_as_pinned_erts() {
    // A reference is a map key, alone and inside a tuple or a list. ERTS
    // orders two references by their number words (`erts_cmp_compound`,
    // utils.c:2239-2290). The probe returns no reference, so both runs
    // print the same text.
    let program = BeamProgram::load(&[REFERENCE_KEY_PROBE]).unwrap();
    let run = |function| {
        program
            .execute("reference_key_probe", function, &[])
            .unwrap()
            .term
    };
    let int = Term::Integer;
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let flat = run("flat");
    assert_eq!(
        flat,
        Term::list_from(vec![
            int(4),
            int(1),
            int(3),
            int(4),
            atom("true"),
            atom("false"),
            atom("true"),
        ])
    );
    assert_eq!(
        run("hash"),
        Term::list_from(vec![int(34), atom("ref"), int(17), atom("true"),])
    );
    assert_eq!(
        run("order"),
        Term::list_from(vec![atom("true"), atom("true"), atom("true"), atom("true"),])
    );
    assert_eq!(
        erl_reference(
            "io:format(\"~p~n~p~n~p~n\", \
             [reference_key_probe:flat(), reference_key_probe:hash(), reference_key_probe:order()])"
        ),
        b"[4,1,3,4,true,false,true]\n[34,ref,17,true]\n[true,true,true,true]\n"
    );
}

#[test]
fn runs_map_operations_with_pid_keys_as_pinned_erts() {
    // A pid is a map key, alone and inside a tuple or a list. ERTS orders
    // two pids by node and number (`erts_cmp_compound`, utils.c:2215-2238).
    // The probe returns no pid, so both runs print the same text.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, PID_KEY_PROBE]).unwrap();
    let run = |function| {
        program
            .execute("pid_key_probe", function, &[])
            .unwrap()
            .term
    };
    let int = Term::Integer;
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        run("flat"),
        Term::list_from(vec![
            int(4),
            int(1),
            int(3),
            int(4),
            atom("true"),
            atom("false"),
            atom("true"),
        ])
    );
    assert_eq!(
        run("hash"),
        Term::list_from(vec![int(34), atom("pid"), int(17), atom("true")])
    );
    assert_eq!(
        run("order"),
        Term::list_from(vec![atom("true"), atom("true"), atom("true"), atom("true"),])
    );
    assert_eq!(
        erl_reference(
            "io:format(\"~p~n~p~n~p~n\", \
             [pid_key_probe:flat(), pid_key_probe:hash(), pid_key_probe:order()])"
        ),
        b"[4,1,3,4,true,false,true]\n[34,pid,17,true]\n[true,true,true,true]\n"
    );
}

const BADMAP_ASSOC_PROBE: &[u8] = include_bytes!("../../fixtures/map/badmap_assoc_probe.beam");

#[test]
fn raises_badmap_for_maps_keys_and_a_map_update_of_a_non_map_like_erts() {
    // `main/0` catches the `{badmap, Term}` of a map update, which the
    // compiler raises with `erlang:error/1`, and of `maps:keys/1`
    // (`erl_map.c:1235-1237`), with the top frame of each stack trace.
    let program = BeamProgram::load(&[BADMAP_ASSOC_PROBE]).unwrap();
    let atom = |name| Term::Atom(program.atom(name).unwrap());
    let not_a_map = atom("not_a_map");
    let badmap = Term::tuple(vec![static_atom("badmap"), not_a_map.clone()]);
    let error_info = Map::from_pairs(
        vec![(atom("module"), atom("erl_stdlib_errors"))],
        program.atoms(),
    )
    .unwrap();
    let expected = Term::list_from([
        Term::tuple(vec![
            static_atom("error"),
            badmap.clone(),
            Term::tuple(vec![
                atom("badmap_assoc_probe"),
                atom("assoc"),
                Term::Integer(1),
                Term::nil(),
            ]),
        ]),
        Term::tuple(vec![
            static_atom("error"),
            badmap,
            Term::tuple(vec![
                atom("maps"),
                atom("keys"),
                Term::list_from([not_a_map]),
                Term::list_from([Term::tuple(vec![atom("error_info"), Term::Map(error_info)])]),
            ]),
        ]),
    ]);
    assert_eq!(
        program
            .execute("badmap_assoc_probe", "main", &[])
            .unwrap()
            .term,
        expected
    );
    assert_eq!(
        erl_reference("io:format(\"~w~n\", [badmap_assoc_probe:main()])"),
        b"[{error,{badmap,not_a_map},{badmap_assoc_probe,assoc,1,[]}},{error,{badmap,not_a_map},{maps,keys,[not_a_map],[{error_info,#{module => erl_stdlib_errors}}]}}]\n"
    );
}

const BIGNUM_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/bignum_key_probe.beam");

#[test]
fn runs_map_operations_with_bignum_keys_as_pinned_erts() {
    // A bignum is a map key, alone and inside a list or a tuple, in a
    // flatmap and in a hashmap of 40 keys. A flatmap orders the keys in the
    // term order, and a hashmap in the order of the internal hash, whose
    // bignum path mixes the sign, the digit count, and the digits
    // (`erl_term_hashing.c:2101-2127`). `encoded/0` returns the external
    // term format of the keys, the pairs, the lookups, and the comparisons,
    // so each byte of the two runs must be equal.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, BIGNUM_KEY_PROBE]).unwrap();
    let encoded = program
        .execute("bignum_key_probe", "encoded", &[])
        .unwrap()
        .term;
    let Term::Binary(bytes) = &encoded else {
        panic!("encoded/0 returns a binary");
    };
    let pbeam: Vec<String> = bytes.bytes().unwrap().iter().map(u8::to_string).collect();
    assert_eq!(
        String::from_utf8(erl_reference(
            "io:format(\"~w~n\", [binary_to_list(bignum_key_probe:encoded())])"
        ))
        .unwrap(),
        format!("[{}]\n", pbeam.join(","))
    );
}

const MAP_IN_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/map_in_key_probe.beam");

#[test]
fn runs_map_operations_with_map_keys_as_pinned_erts() {
    // A map is a map key, alone and inside a list, a tuple, or another map,
    // in a flatmap and in a hashmap of 52 keys. The keys include flatmaps
    // and hashmaps of 40 and 41 keys. A hashmap orders the keys in the order
    // of the internal hash, whose map path mixes the flatmap pairs or the
    // hashmap nodes. `encoded/0` returns the external term format of the
    // keys, the pairs, the lookups, and the comparisons, so each byte of the
    // two runs must be equal.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, MAP_IN_KEY_PROBE]).unwrap();
    let encoded = program
        .execute("map_in_key_probe", "encoded", &[])
        .unwrap()
        .term;
    let Term::Binary(bytes) = &encoded else {
        panic!("encoded/0 returns a binary");
    };
    let pbeam: Vec<String> = bytes.bytes().unwrap().iter().map(u8::to_string).collect();
    assert_eq!(
        String::from_utf8(erl_reference(
            "io:format(\"~w~n\", [binary_to_list(map_in_key_probe:encoded())])"
        ))
        .unwrap(),
        format!("[{}]\n", pbeam.join(","))
    );
}

const BINARY_IN_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/binary_in_key_probe.beam");

#[test]
fn runs_map_operations_with_binaries_in_compound_keys_as_pinned_erts() {
    // Tuple, list, and map keys hold a heap binary, a refc binary, a
    // sub-binary, an unaligned sub-binary, and a bitstring of 3 bits, in a
    // flatmap of 12 keys and in a hashmap of 53 keys. A hashmap orders the
    // keys in the order of the internal hash, whose binary path mixes the
    // size in bits and the bytes. `encoded/0` returns the external term
    // format of the keys, the pairs, the lookups, and the comparisons, so
    // each byte of the two runs must be equal.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, BINARY_IN_KEY_PROBE]).unwrap();
    let encoded = program
        .execute("binary_in_key_probe", "encoded", &[])
        .unwrap()
        .term;
    let Term::Binary(bytes) = &encoded else {
        panic!("encoded/0 returns a binary");
    };
    let pbeam: Vec<String> = bytes.bytes().unwrap().iter().map(u8::to_string).collect();
    assert_eq!(
        String::from_utf8(erl_reference(
            "io:format(\"~w~n\", [binary_to_list(binary_in_key_probe:encoded())])"
        ))
        .unwrap(),
        format!("[{}]\n", pbeam.join(","))
    );
}

const BITSTRING_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/bitstring_key_probe.beam");

#[test]
fn runs_map_operations_with_bitstring_keys_as_pinned_erts() {
    // Top-level bitstring keys of 1 to 403 bits, a sub-bitstring of a refc
    // binary, and unaligned sub-bitstrings, in a flatmap of 13 keys and in a
    // hashmap of 46 keys. The binary path of `make_internal_hash` hashes the
    // trailing bits of a bitstring. `encoded/0` returns the external term
    // format of the keys, the pairs, and the lookups, so each byte of the
    // two runs must be equal.
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, BITSTRING_KEY_PROBE]).unwrap();
    let encoded = program
        .execute("bitstring_key_probe", "encoded", &[])
        .unwrap()
        .term;
    let Term::Binary(bytes) = &encoded else {
        panic!("encoded/0 returns a binary");
    };
    let pbeam: Vec<String> = bytes.bytes().unwrap().iter().map(u8::to_string).collect();
    assert_eq!(
        String::from_utf8(erl_reference(
            "io:format(\"~w~n\", [binary_to_list(bitstring_key_probe:encoded())])"
        ))
        .unwrap(),
        format!("[{}]\n", pbeam.join(","))
    );
}

const FUN_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/fun_key_probe.beam");

#[test]
fn runs_map_operations_with_fun_keys_as_pinned_erts() {
    // A fun is a map key, alone and inside a tuple or a list. A hashmap
    // hashes a local fun by its free variable count, index, old unique
    // value, module, and free variables (`erl_term_hashing.c:1987-2010`).
    // The module atom index differs between ERTS and PBeam, so the
    // hashmap pair order is exact only for PBeam on the CI host; the ERTS
    // run checks the same set of values. The probe returns no fun.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let maps = pinned_beam("maps");
    let program = BeamProgram::load(&[&erlang, &lists, &maps, FUN_KEY_PROBE]).unwrap();
    let int = Term::Integer;
    let ints = |values: &[i64]| Term::list_from(values.iter().map(|&v| int(v)));
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let run = |function| {
        program
            .execute("fun_key_probe", function, &[])
            .unwrap()
            .term
    };
    let hash_order = [
        33, 32, 3, 11, 8, 30, 22, 6, 21, 18, 16, 35, 37, 5, 31, 28, 14, 1, 26, 38, 23, 20, 10, 17,
        39, 9, 25, 13, 24, 2, 29, 34, 27, 12, 19, 7, 4, 36, 15, 40,
    ];
    let nested_order = [
        9, 11, 12, 6, 2, -2, 30, 31, 25, 27, 34, -3, 1, 21, 5, 26, 16, 3, 24, 20, 32, 23, 10, 17,
        7, 13, 28, 4, 8, -1, 33, 15, 14, 18, 19, 22, 29, -4,
    ];
    assert_eq!(
        run("flat"),
        Term::list_from(vec![
            int(5),
            int(1),
            int(2),
            int(3),
            int(4),
            int(5),
            atom("false"),
            ints(&[1, 2, 3, 4, 5]),
        ])
    );
    assert_eq!(
        run("hash"),
        Term::list_from(vec![int(40), int(17), atom("false"), ints(&hash_order)])
    );
    assert_eq!(
        run("nested"),
        Term::list_from(vec![int(38), int(-2), ints(&nested_order)])
    );
    assert_eq!(
        run("order"),
        Term::list_from(vec![
            atom("true"),
            atom("false"),
            atom("true"),
            atom("true"),
            atom("true"),
        ])
    );
    assert_eq!(
        run("external"),
        Term::list_from(vec![int(1), int(2), ints(&[3, 1, 2])])
    );
    assert_eq!(
        erl_reference(
            "io:format(\"~w~n~w~n~w~n~w~n~w~n\", [fun_key_probe:flat(), \
             lists:sort(lists:last(fun_key_probe:hash())), \
             lists:sort(lists:last(fun_key_probe:nested())), \
             fun_key_probe:order(), fun_key_probe:external()])"
        ),
        b"[5,1,2,3,4,5,false,[1,2,3,4,5]]\n\
          [1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40]\n\
          [-4,-3,-2,-1,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34]\n\
          [true,false,true,true,true]\n[1,2,[3,1,2]]\n"
    );
}

const PORT_KEY_PROBE: &[u8] = include_bytes!("../../fixtures/map/port_key_probe.beam");

#[test]
fn runs_map_operations_with_port_keys_as_pinned_erts() {
    // A port is a map key, alone and inside a tuple, a list, or a map key.
    // A hashmap hashes a port by its immediate word
    // (`erl_term_hashing.c:2211-2213`), which holds the port number spread
    // over the port table index bits (`erts_ptab_make_id`,
    // `erl_ptab.h:320-326`). The probe returns no port.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let maps = pinned_beam("maps");
    let program = BeamProgram::load(&[&erlang, &lists, &maps, PORT_KEY_PROBE]).unwrap();
    let int = Term::Integer;
    let ints = |values: &[i64]| Term::list_from(values.iter().map(|&v| int(v)));
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let run = |function| {
        program
            .execute("port_key_probe", function, &[])
            .unwrap()
            .term
    };
    let hash_order = [
        20, 25, 27, 1, 22, 40, 17, 37, 15, 12, 10, 36, 14, 30, 32, 13, 19, 28, 18, 38, 39, 2, 21,
        7, 4, 26, 29, 23, 5, 24, 3, 34, 8, 33, 6, 11, 31, 35, 16, 9,
    ];
    let removed: Vec<i64> = hash_order.iter().copied().filter(|&n| n != 17).collect();
    let nested_order = [
        11, 5, -2, 20, 31, 6, -1, 17, 15, 33, 19, 13, 28, 32, 9, 29, 34, 10, 25, 14, 3, -3, -4,
        102, 22, 21, 30, 1, 7, 23, 27, 24, 18, 2, 101, 26, 16, 4, 12, 8,
    ];
    assert_eq!(
        run("flat"),
        Term::list_from(vec![
            int(8),
            int(1),
            int(5),
            int(6),
            atom("false"),
            atom("false"),
            int(7),
            ints(&[4, 3, 1, 2, 7, 5, 6, 8]),
            ints(&[4, 3, 1, 7, 5, 6, 8]),
        ])
    );
    assert_eq!(
        run("hash"),
        Term::list_from(vec![
            int(40),
            int(17),
            atom("false"),
            atom("false"),
            ints(&hash_order),
            ints(&removed),
        ])
    );
    assert_eq!(
        run("nested"),
        Term::list_from(vec![
            int(40),
            int(-2),
            int(5),
            int(102),
            ints(&nested_order)
        ])
    );
    // PBeam spreads a port number over the 16 index bits of the default
    // ERTS port table (`ERTS_DEFAULT_MAX_PORTS`, `erl_port.h:41`). ERTS
    // raises the table size to the host file limit (`io.c:2997-3006`), so
    // the ERTS run checks the order only on a host with `port_limit` 65536,
    // such as the Linux CI host.
    let high = "[8206,8195,8219,8221,8207,8222,8202,8227,8217,8198,8194,8218,8203,8200,8190,8216,8208,8225,8229,8197,8199,8223,8220,8191,8192,8201,8210,8196,8205,8213,8214,8204,8226,8215,8228,8211,8193,8209,8224,8212]";
    let high_order: Vec<i64> = high[1..high.len() - 1]
        .split(',')
        .map(|n| n.parse().unwrap())
        .collect();
    assert_eq!(run("high"), ints(&high_order));
    let reference = erl_reference(
        "io:format(\"~w~n~w~n\", [erlang:system_info(port_limit), port_key_probe:high()])",
    );
    if reference.starts_with(b"65536\n") {
        assert_eq!(reference, format!("65536\n{high}\n").into_bytes());
    }
    let (yes, no) = (atom("true"), atom("false"));
    assert_eq!(
        run("order"),
        Term::list_from(vec![
            yes.clone(),
            no.clone(),
            yes.clone(),
            yes.clone(),
            no,
            yes.clone(),
            yes.clone(),
            yes,
        ])
    );
    assert_eq!(
        erl_reference(
            "io:format(\"~w~n~w~n~w~n~w~n\", [port_key_probe:flat(), \
             port_key_probe:hash(), port_key_probe:nested(), \
             port_key_probe:order()])"
        ),
        b"[8,1,5,6,false,false,7,[4,3,1,2,7,5,6,8],[4,3,1,7,5,6,8]]\n\
          [40,17,false,false,[20,25,27,1,22,40,17,37,15,12,10,36,14,30,32,13,19,28,18,38,39,2,21,7,4,26,29,23,5,24,3,34,8,33,6,11,31,35,16,9],[20,25,27,1,22,40,37,15,12,10,36,14,30,32,13,19,28,18,38,39,2,21,7,4,26,29,23,5,24,3,34,8,33,6,11,31,35,16,9]]\n\
          [40,-2,5,102,[11,5,-2,20,31,6,-1,17,15,33,19,13,28,32,9,29,34,10,25,14,3,-3,-4,102,22,21,30,1,7,23,27,24,18,2,101,26,16,4,12,8]]\n\
          [true,false,true,true,false,true,true,true]\n"
    );
}

const PID_ORDER_PROBE: &[u8] = include_bytes!("../../fixtures/map/pid_order_probe.beam");

#[test]
fn orders_a_hashmap_of_pid_keys_as_pinned_erts() {
    // A hashmap hashes a pid by its immediate word, which holds the pid
    // number spread over the process table index bits (`make_internal_pid`,
    // `erl_node_container_utils.h:124-126`; `erl_ptab.h:320-326`). The probe
    // returns no pid.
    let erlang = pinned_beam("erlang");
    let lists = pinned_beam("lists");
    let maps = pinned_beam("maps");
    let program = BeamProgram::load(&[&erlang, &lists, &maps, PID_ORDER_PROBE]).unwrap();
    let order = [
        36, 35, 31, 15, 30, 25, 26, 19, 10, 2, 13, 1, 16, 24, 28, 4, 3, 32, 11, 17, 18, 6, 22, 23,
        14, 37, 40, 39, 8, 12, 7, 20, 27, 9, 34, 38, 33, 5, 29, 21,
    ];
    let removed: Vec<i64> = order.iter().copied().filter(|&n| n != 17).collect();
    let ints = |values: &[i64]| Term::list_from(values.iter().map(|&v| Term::Integer(v)));
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    assert_eq!(
        program
            .execute("pid_order_probe", "hash", &[])
            .unwrap()
            .term,
        Term::list_from(vec![
            Term::Integer(40),
            Term::Integer(17),
            atom("false"),
            atom("false"),
            ints(&order),
            ints(&removed),
        ])
    );
    let expected: &[u8] = b"[40,17,false,false,[36,35,31,15,30,25,26,19,10,2,13,1,16,24,28,4,3,32,11,17,18,6,22,23,14,37,40,39,8,12,7,20,27,9,34,38,33,5,29,21],[36,35,31,15,30,25,26,19,10,2,13,1,16,24,28,4,3,32,11,18,6,22,23,14,37,40,39,8,12,7,20,27,9,34,38,33,5,29,21]]\n";
    assert_eq!(
        erl_reference("io:format(\"~w~n\", [pid_order_probe:hash()])"),
        expected
    );
}

const MAPS_FUN_PROBE: &[u8] = include_bytes!("../../fixtures/map/maps_fun_probe.beam");

#[test]
fn runs_maps_get_and_is_key_through_a_fun_and_apply_as_pinned_erts() {
    // A fun call, `apply`, and `apply_last` of `maps:get/2` and
    // `maps:is_key/2` run the `maps_get_2` and `maps_is_key_2` BIFs
    // (`erl_map.c:250-264`, `erl_map.c:1185-1191`). The stack frame of each
    // error holds the `error_info` of `erl_stdlib_errors`
    // (`beam_common.c:939-991`).
    let erlang = pinned_beam("erlang");
    let program = BeamProgram::load(&[&erlang, MAPS_FUN_PROBE]).unwrap();
    let atom = |name: &str| Term::Atom(program.atom(name).unwrap());
    let map =
        Term::Map(Map::from_pairs(vec![(atom("a"), Term::Integer(1))], &AtomTable::new()).unwrap());
    let error = |reason: &str, value: Term, name: &str, arguments: Vec<Term>| {
        let info = Term::Map(
            Map::from_pairs(
                vec![(atom("module"), atom("erl_stdlib_errors"))],
                &AtomTable::new(),
            )
            .unwrap(),
        );
        Term::tuple(vec![
            Term::tuple(vec![atom(reason), value]),
            Term::tuple(vec![
                atom("maps"),
                atom(name),
                Term::list_from(arguments),
                Term::list_from(vec![Term::tuple(vec![atom("error_info"), info])]),
            ]),
        ])
    };
    let badkey = error("badkey", atom("b"), "get", vec![atom("b"), map]);
    assert_eq!(
        program.execute("maps_fun_probe", "main", &[]).unwrap().term,
        Term::list_from(vec![
            Term::Integer(1),
            atom("true"),
            atom("false"),
            Term::Integer(1),
            atom("false"),
            badkey.clone(),
            error("badmap", atom("x"), "get", vec![atom("a"), atom("x")]),
            error("badmap", atom("x"), "is_key", vec![atom("a"), atom("x")]),
            badkey.clone(),
            error("badmap", atom("x"), "is_key", vec![atom("a"), atom("x")]),
            Term::Integer(1),
            badkey,
        ])
    );
    let expected: &[u8] = b"[1,true,false,1,false,{{badkey,b},{maps,get,[b,#{a => 1}],[{error_info,#{module => erl_stdlib_errors}}]}},{{badmap,x},{maps,get,[a,x],[{error_info,#{module => erl_stdlib_errors}}]}},{{badmap,x},{maps,is_key,[a,x],[{error_info,#{module => erl_stdlib_errors}}]}},{{badkey,b},{maps,get,[b,#{a => 1}],[{error_info,#{module => erl_stdlib_errors}}]}},{{badmap,x},{maps,is_key,[a,x],[{error_info,#{module => erl_stdlib_errors}}]}},1,{{badkey,b},{maps,get,[b,#{a => 1}],[{error_info,#{module => erl_stdlib_errors}}]}}]\n";
    assert_eq!(
        erl_reference("io:format(\"~w~n\", [maps_fun_probe:main()])"),
        expected
    );
}
