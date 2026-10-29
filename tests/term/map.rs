use std::{hint::black_box, rc::Rc, sync::Arc, time::Instant};

use super::{
    Child, Direction, Drain, HAMT_MAX_LEVEL, Hamt, Insert, Map, Node, Repr, encode_path, map_hash,
};
use crate::term::{
    AtomTable, Binary, MatchState, Reference, Term, internal_hash, test_atom as atom,
    test_atom_table,
};

fn integers(keys: impl IntoIterator<Item = i64>) -> Map {
    Map::from_pairs(
        keys.into_iter()
            .map(|key| (Term::Integer(key), atom("v")))
            .collect(),
        &test_atom_table(),
    )
    .unwrap()
}

fn keys(map: &Map) -> Vec<Term> {
    map.iter().map(|(key, _)| key.clone()).collect()
}

fn list(keys: &[i64]) -> Vec<Term> {
    keys.iter().map(|key| Term::Integer(*key)).collect()
}

/// Return a match context, the one term that a map does not hold: the
/// interpreter gives a runtime error before one reaches a map operation.
fn match_context() -> Term {
    Term::MatchContext(Arc::new(MatchState::new(Binary::from(b"a"))))
}

#[test]
fn hashes_small_integers_as_erts() {
    assert_eq!(map_hash(&Term::Integer(1)), Some(7_938_660_269_479_274_665));
    assert_eq!(
        map_hash(&Term::Integer(-1)),
        Some(7_256_831_767_414_464_289)
    );
    assert!(map_hash(&Term::Integer((1 << 59) - 1)).is_some());
    assert!(map_hash(&Term::Integer(-(1 << 59))).is_some());
    // A bignum key hashes with the bignum path of the internal hash.
    assert_eq!(
        map_hash(&Term::Integer(1 << 59)),
        Some(10_054_563_828_903_259_082)
    );
    assert_eq!(
        map_hash(&Term::Integer(-(1 << 59) - 1)),
        Some(12_317_164_193_119_007_090)
    );
}

#[test]
fn hashes_atoms_by_their_index_as_erts() {
    // true has index 1: its ERTS word is (1 << 6) | 0xB = 0x4B.
    assert_eq!(
        map_hash(&Term::static_atom("true")),
        Some(14_558_535_449_716_669_319)
    );
    // abs_path has index 26. ERTS 29.1.1: its internal hash.
    assert_eq!(
        map_hash(&Term::static_atom("abs_path")),
        Some(17_379_220_480_677_090_226)
    );
}

#[test]
fn keeps_flatmap_keys_in_order_and_replaces_values() {
    // A fresh table for this test makes z before a, so z has the lower
    // atom index. A table shared with other tests could make a first.
    let mut table = AtomTable::new();
    let atom = |table: &AtomTable, name: &str| Term::Atom(table.put(name).unwrap());
    let mut map = Map::from_pairs(
        vec![
            (atom(&mut table, "z"), atom(&mut table, "z")),
            (atom(&mut table, "a"), atom(&mut table, "old")),
            (Term::Integer(2), atom(&mut table, "v")),
            (Term::Integer(i64::MIN), atom(&mut table, "v")),
            (atom(&mut table, "a"), atom(&mut table, "a")),
        ],
        &table,
    )
    .unwrap();
    assert_eq!(
        keys(&map),
        [
            Term::Integer(i64::MIN),
            Term::Integer(2),
            atom(&mut table, "z"),
            atom(&mut table, "a")
        ]
    );
    // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path],
    // because true has index 1 and abs_path has index 26.
    let static_atoms = Map::from_pairs(
        vec![
            (Term::static_atom("abs_path"), atom(&mut table, "v")),
            (Term::static_atom("true"), atom(&mut table, "v")),
            (Term::Integer(1), atom(&mut table, "v")),
        ],
        &table,
    )
    .unwrap();
    assert_eq!(
        keys(&static_atoms),
        [
            Term::Integer(1),
            Term::static_atom("true"),
            Term::static_atom("abs_path")
        ]
    );
    assert_eq!(
        map.get(&atom(&mut table, "a")),
        Some(&atom(&mut table, "a"))
    );
    assert_eq!(map.get(&atom(&mut table, "b")), None);
    assert_eq!(map.get(&Term::tuple(vec![])), None);
    map.insert(atom(&mut table, "z"), atom(&mut table, "new"), &table)
        .unwrap();
    assert_eq!(
        map.get(&atom(&mut table, "z")),
        Some(&atom(&mut table, "new"))
    );
    assert_eq!(map.len(), 4);
    // A key type that the flatmap does not support leaves the map unchanged.
    let unsupported = Term::tuple(vec![match_context()]);
    assert_eq!(
        map.insert(unsupported.clone(), atom(&mut table, "v"), &table),
        Err(unsupported)
    );
    assert_eq!(map.len(), 4);
}

#[test]
fn orders_atoms_in_list_keys_by_name_in_a_flatmap() {
    // A fresh table for this test makes z before a, so z has the lower atom index.
    let mut table = AtomTable::new();
    let atom = |table: &AtomTable, name: &str| Term::Atom(table.put(name).unwrap());
    let (z, a, v) = (
        atom(&mut table, "z"),
        atom(&mut table, "a"),
        atom(&mut table, "v"),
    );
    let list = |term: &Term| Term::list_from(vec![term.clone()]);
    let map = Map::from_pairs(
        vec![
            (list(&z), v.clone()),
            (a.clone(), v.clone()),
            (list(&a), v.clone()),
            (z.clone(), v.clone()),
        ],
        &table,
    )
    .unwrap();
    // erts_cmp_flatmap_keys: two atoms by index, two lists in the term order.
    assert_eq!(keys(&map), [z.clone(), a.clone(), list(&a), list(&z)]);
    assert_eq!(map.get(&list(&z)), Some(&v));
}

#[test]
fn hashes_tuple_keys_with_the_internal_hash() {
    // {}, {1, {a}}, [{}], and [1|{2}]: a tuple, also in a list or as an improper list tail.
    let tuple = Term::tuple(vec![Term::Integer(1), Term::tuple(vec![atom("a")])]);
    let keys = [
        Term::tuple(vec![]),
        tuple,
        Term::list_from(vec![Term::tuple(vec![])]),
        Term::list_with_tail(vec![Term::Integer(1)], Term::tuple(vec![Term::Integer(2)])),
    ];
    for key in keys {
        assert_eq!(map_hash(&key), Some(internal_hash(&key)));
    }
    assert_eq!(
        map_hash(&Term::tuple(vec![])),
        Some(4_510_301_282_025_278_020)
    );
}

#[test]
fn hashes_an_alias_reference_with_its_three_numbers() {
    assert_eq!(map_hash(&Term::nil()), Some(internal_hash(&Term::nil())));
    // A PBeam alias reference hashes its three number words, so it is a
    // supported map key alone and inside a list, tuple, or map
    // (docs/erts-differences.md, "Map keys").
    let alias = || Term::Reference(Reference(0).pid_reference());
    assert_eq!(map_hash(&alias()), Some(internal_hash(&alias())));
    let alias_map = || {
        Term::Map(Map::from_pairs(vec![(Term::Integer(1), alias())], &test_atom_table()).unwrap())
    };
    assert_eq!(
        map_hash(&Term::tuple(vec![alias()])),
        Some(internal_hash(&Term::tuple(vec![alias()])))
    );
    assert_eq!(
        map_hash(&Term::list_with_tail(vec![Term::Integer(1)], alias())),
        Some(internal_hash(&Term::list_with_tail(
            vec![Term::Integer(1)],
            alias()
        )))
    );
    assert_eq!(
        map_hash(&Term::tuple(vec![Term::Integer(1), alias_map()])),
        Some(internal_hash(&Term::tuple(vec![
            Term::Integer(1),
            alias_map()
        ])))
    );
}

#[test]
fn rejects_list_and_tuple_keys_with_unsupported_elements() {
    let port_map = || {
        Term::Map(
            Map::from_pairs(
                vec![(Term::Integer(1), match_context())],
                &test_atom_table(),
            )
            .unwrap(),
        )
    };
    let unsupported = [
        // A key that is not supported.
        match_context(),
        Term::list_from(vec![Term::Integer(1), port_map()]),
        Term::list_from(vec![Term::list_from(vec![port_map()])]),
        // A tuple with an element that is not supported.
        Term::tuple(vec![match_context()]),
        Term::tuple(vec![Term::Integer(1), port_map()]),
        // An improper list tail that is not supported.
        Term::list_with_tail(vec![Term::Integer(1)], port_map()),
        Term::list_with_tail(vec![Term::Integer(1)], match_context()),
        Term::list_from(vec![Term::list_with_tail(
            vec![Term::Integer(1)],
            port_map(),
        )]),
    ];
    for key in unsupported {
        assert_eq!(map_hash(&key), None);
        let mut flat = integers([1]);
        assert_eq!(
            flat.insert(key.clone(), atom("v"), &test_atom_table()),
            Err(key.clone())
        );
        assert_eq!(keys(&flat), list(&[1]));
        let mut hash = integers(1..=33);
        assert_eq!(
            hash.insert(key.clone(), atom("v"), &test_atom_table()),
            Err(key.clone())
        );
        assert_eq!(hash.len(), 33);
        assert_eq!(hash.get(&key), None);
    }
}

#[test]
fn hashes_improper_list_keys_with_their_tail_as_erts() {
    // ERTS 29.1.1: maps:keys of #{[1|2] => v, [1,2|3] => v, [[1|2]|3] => v,
    // [true|false] => v, [256|1] => v, [[1]|2] => v, [[]|-1] => v, 1 => v, ..., 26 => v}.
    let improper = |elements: Vec<Term>, tail| Term::list_with_tail(elements, tail);
    let int = Term::Integer;
    let one_two = || improper(vec![int(1)], int(2));
    let one_two_three = || improper(vec![int(1), int(2)], int(3));
    let nested = || improper(vec![one_two()], int(3));
    let booleans = || improper(vec![Term::static_atom("true")], Term::static_atom("false"));
    let large = || improper(vec![int(256)], int(1));
    let list_head = || improper(vec![Term::list_from(vec![int(1)])], int(2));
    let nil_head = || improper(vec![Term::nil()], int(-1));
    let lists = [
        one_two(),
        one_two_three(),
        nested(),
        booleans(),
        large(),
        list_head(),
        nil_head(),
    ];
    let map = Map::from_pairs(
        lists
            .into_iter()
            .chain((1..=26).map(int))
            .map(|key| (key, atom("v")))
            .collect(),
        &test_atom_table(),
    )
    .unwrap();
    assert_eq!(
        keys(&map),
        [
            int(18),
            list_head(),
            nested(),
            int(4),
            large(),
            int(12),
            int(19),
            int(13),
            int(2),
            int(7),
            int(8),
            int(10),
            int(23),
            int(9),
            booleans(),
            one_two(),
            int(15),
            one_two_three(),
            int(1),
            int(25),
            int(20),
            int(6),
            int(11),
            int(17),
            int(24),
            int(14),
            nil_head(),
            int(3),
            int(16),
            int(21),
            int(5),
            int(26),
            int(22),
        ]
    );
    assert_eq!(map.get(&one_two_three()), Some(&atom("v")));
    assert_eq!(map.get(&improper(vec![int(1), int(2)], int(4))), None);
}

#[test]
fn becomes_a_hashmap_with_the_erts_order_above_32_keys() {
    let mut map = integers(1..=32);
    assert_eq!(keys(&map), list(&(1..=32).collect::<Vec<_>>()));
    // Replacing a value in a full flatmap keeps the flatmap.
    map.insert(Term::Integer(32), Term::Integer(0), &test_atom_table())
        .unwrap();
    assert_eq!(keys(&map), list(&(1..=32).collect::<Vec<_>>()));
    map.insert(Term::Integer(33), atom("v"), &test_atom_table())
        .unwrap();
    assert_eq!(map.len(), 33);
    // ERTS 29.1.1: maps:keys of 1..33.
    assert_eq!(
        keys(&map),
        list(&[
            18, 4, 12, 19, 29, 13, 2, 7, 31, 8, 10, 23, 9, 15, 32, 1, 25, 28, 20, 6, 11, 17, 24,
            14, 33, 3, 16, 30, 21, 5, 27, 26, 22
        ])
    );
    assert_eq!(map.get(&Term::Integer(32)), Some(&Term::Integer(0)));
    map.insert(Term::Integer(33), Term::Integer(1), &test_atom_table())
        .unwrap();
    assert_eq!(map.get(&Term::Integer(33)), Some(&Term::Integer(1)));
    assert_eq!(map.len(), 33);
    assert_eq!(map.get(&Term::Integer(34)), None);
    assert_eq!(map.get(&atom("a")), None);

    // ERTS 29.1.1: maps:keys of 1..64, where every root slot is used.
    let full = integers((1..=64).rev());
    assert_eq!(
        keys(&full),
        list(&[
            54, 18, 4, 34, 46, 52, 12, 19, 41, 29, 13, 42, 55, 2, 7, 38, 56, 61, 31, 8, 44, 10, 23,
            9, 49, 64, 15, 35, 36, 32, 58, 1, 25, 28, 60, 20, 6, 43, 57, 11, 48, 17, 24, 37, 40,
            14, 33, 53, 3, 16, 30, 62, 21, 5, 27, 47, 63, 26, 51, 22, 50, 45, 59, 39
        ])
    );
    assert!((1..=64).all(|key| full.get(&Term::Integer(key)).is_some()));

    // The first three keys have hashes with the same low 60 bits, and so do the next two.
    // Each group splits only at level 15. ERTS 29.1.1: maps:keys of this map.
    let deep = [
        413_866_500_823_436_792,
        -101_714_049_538_469_044,
        -74_925_715_755_171_563,
        329_898_321_527_737_274,
        446_694_106_686_689_078,
    ];
    assert_eq!(
        map_hash(&Term::Integer(deep[0])),
        Some(0x6600_a35a_0999_50d8)
    );
    assert_eq!(
        map_hash(&Term::Integer(deep[1])),
        Some(0x7600_a35a_0999_50d8)
    );
    assert_eq!(
        map_hash(&Term::Integer(deep[2])),
        Some(0xf600_a35a_0999_50d8)
    );
    assert_eq!(
        map_hash(&Term::Integer(deep[3])),
        Some(0x3b04_04f2_b094_90b8)
    );
    assert_eq!(
        map_hash(&Term::Integer(deep[4])),
        Some(0x7b04_04f2_b094_90b8)
    );
    let deep60 = integers(deep.into_iter().chain(100..=129));
    assert_eq!(
        keys(&deep60),
        list(&[
            128, 123, 110, 114, 100, 113, 104, 105, 115, 127, 116, 111, 101, 112, 125, 103, 118,
            109, 106, 117, deep[3], deep[4], deep[0], deep[1], deep[2], 126, 107, 108, 119, 122,
            129, 124, 121, 102, 120
        ])
    );
    assert!(
        deep.iter()
            .all(|key| deep60.get(&Term::Integer(*key)).is_some())
    );

    // ERTS 29.1.1: the small-integer limits in a hashmap.
    let (min, max) = (-(1i64 << 59), (1i64 << 59) - 1);
    let edges = integers([-1, min, max].into_iter().chain(1..=30));
    assert_eq!(
        keys(&edges),
        list(&[
            18, 4, -1, 12, 19, 29, 13, 2, 7, min, 8, 10, 23, 9, 15, 1, 25, 28, 20, 6, max, 11, 17,
            24, 14, 3, 16, 30, 21, 5, 27, 26, 22
        ])
    );
}

#[test]
fn puts_atom_and_bignum_keys_in_a_hashmap() {
    let a = atom("a");
    // The 33rd key turns a flatmap with an atom key into a hashmap.
    let mut map = integers(1..=31);
    map.insert(a.clone(), a.clone(), &test_atom_table())
        .unwrap();
    map.insert(Term::Integer(33), a.clone(), &test_atom_table())
        .unwrap();
    assert_eq!(map.len(), 33);
    assert_eq!(map.get(&a), Some(&a));
    assert_eq!(map.get(&atom("b")), None);
    // An atom key goes into a hashmap.
    let mut map = integers(1..=33);
    map.insert(a.clone(), Term::Integer(1), &test_atom_table())
        .unwrap();
    map.insert(a.clone(), Term::Integer(2), &test_atom_table())
        .unwrap();
    assert_eq!(map.len(), 34);
    assert_eq!(map.get(&a), Some(&Term::Integer(2)));
    // A bignum key goes into a hashmap, in a list or a tuple too.
    let big = Term::Integer(1 << 59);
    let nested = Term::tuple(vec![Term::list_from(vec![big.clone()])]);
    for key in [big.clone(), nested] {
        map.insert(key.clone(), a.clone(), &test_atom_table())
            .unwrap();
        assert_eq!(map.get(&key), Some(&a));
    }
    // A flatmap with a bignum key turns into a hashmap.
    let mut map = integers(1..=31);
    map.insert(big.clone(), a.clone(), &test_atom_table())
        .unwrap();
    map.insert(Term::Integer(33), a.clone(), &test_atom_table())
        .unwrap();
    assert_eq!(map.len(), 33);
    assert_eq!(map.get(&big), Some(&a));
}

/// Return the map of a `Map` term.
fn map_of(term: &Term) -> &Map {
    let Term::Map(map) = term else {
        panic!("not a map: {term:?}")
    };
    map
}

#[test]
fn shares_the_form_of_a_map_between_the_copies_of_a_term() {
    // A copy of the term, also a copy of a term that holds the map, gets the same form.
    for size in [2, 40] {
        let map = Term::Map(integers(1..=size));
        let tuple = Term::tuple(vec![Term::list_from(vec![map.clone()])]);
        let tuple = tuple.clone();
        let Term::Tuple(copied) = &tuple else {
            unreachable!()
        };
        let copied = copied[0].list_head().unwrap();
        for copy in [&map.clone(), copied] {
            assert!(Arc::ptr_eq(map_of(&map).arc(), map_of(copy).arc()));
            assert_eq!(copy, &map);
        }
    }
}

#[test]
fn copies_a_shared_form_before_a_change() {
    let change = |change: fn(&mut Map)| {
        for size in [2, 32, 40] {
            let map = integers(1..=size);
            let mut copy = map.clone();
            change(&mut copy);
            assert!(!Arc::ptr_eq(map.arc(), copy.arc()));
            assert_eq!(keys(&map), keys(&integers(1..=size)));
            assert!(map.iter().all(|(_, value)| *value == atom("v")));
            assert_eq!(map.is_hashmap(), size > 32);
            // The copy that changed is only owner of its form now, so a second change does not copy it.
            let form = Arc::as_ptr(copy.arc());
            change(&mut copy);
            assert_eq!(Arc::as_ptr(copy.arc()), form);
        }
    };
    change(|map| {
        map.update(
            vec![(Insert::Set, Term::Integer(1), atom("w"))],
            &test_atom_table(),
        )
        .unwrap();
    });
    change(|map| {
        map.insert(Term::Integer(2), atom("w"), &test_atom_table())
            .unwrap();
    });
    change(|map| {
        map.insert(Term::Integer(-1), atom("w"), &test_atom_table())
            .unwrap();
    });

    // A copy of a flatmap of 32 keys that gets the 33rd key is a hashmap. The other copy stays
    // a flatmap of 32 keys.
    let flat = integers(1..=32);
    let mut grown = flat.clone();
    grown
        .insert(Term::Integer(33), atom("v"), &test_atom_table())
        .unwrap();
    assert_eq!((flat.len(), flat.is_hashmap()), (32, false));
    assert_eq!((grown.len(), grown.is_hashmap()), (33, true));
}

/// Check that `copy` holds a new node for each node of `map` on the path of
/// `hash`, and the same node for each node of `map` off the path. Return the
/// number of shared nodes.
fn check_path_copy(map: &Node, copy: &Node, hash: u64, level: u32) -> usize {
    let mut shared = 0;
    for slot in 0..16 {
        let (Some(Child::Node(old)), Some(Child::Node(new))) = (map.child(slot), copy.child(slot))
        else {
            continue;
        };
        if slot == super::slot(hash, level) {
            assert!(!Arc::ptr_eq(old, new), "level {level} slot {slot}");
            shared += check_path_copy(old, new, hash, level + 1);
        } else {
            assert!(Arc::ptr_eq(old, new), "level {level} slot {slot}");
            shared += 1;
        }
    }
    shared
}

#[test]
fn copies_only_the_hashmap_nodes_on_the_path_of_a_change() {
    // ERTS 29.1.1 `erts_hashmap_insert` copies the nodes on the path of the key, and the new
    // map holds each other node of the old map (`erl_map.c:2477-2503`, `2659-2792`).
    let map = integers(1..=1000);
    let change = |key: i64, shared: usize, change: fn(&mut Map, Term)| {
        let mut copy = map.clone();
        change(&mut copy, Term::Integer(key));
        let (Repr::Hash(old), Repr::Hash(new)) = (&**map.arc(), &**copy.arc()) else {
            panic!("not a hashmap")
        };
        let hash = map_hash(&Term::Integer(key)).unwrap();
        // The keys 1..1000 fill the 16 slots of the root with nodes: 15 are off the path. The
        // other shared nodes are below the root.
        assert_eq!(check_path_copy(&old.root, &new.root, hash, 0), shared);
        assert_eq!(
            map.get(&Term::Integer(key)),
            (key <= 1000).then_some(&atom("v"))
        );
        assert_eq!(copy.get(&Term::Integer(key)), Some(&atom("w")));
    };
    change(500, 28, |map, key| {
        map.insert(key, atom("w"), &test_atom_table()).unwrap();
    });
    change(2000, 29, |map, key| {
        map.insert(key, atom("w"), &test_atom_table()).unwrap();
    });
    change(500, 28, |map, key| {
        map.update(vec![(Insert::Set, key, atom("w"))], &test_atom_table())
            .unwrap();
    });
}

#[test]
fn drains_the_pairs_of_a_map_only_from_the_last_copy() {
    let mut pending = Vec::new();
    for size in [2, 40] {
        let mut map = integers(1..=size);
        let mut copy = map.clone();
        map.drain_into(&mut pending);
        assert_eq!(pending, [] as [crate::term::Term; 0]);
        assert_eq!(copy.len(), size as usize);
        // `Term::drop` drops the map after `drain_into`.
        drop(map);
        copy.drain_into(&mut pending);
        assert_eq!(pending.len(), 2 * size as usize);
        // The last holder takes the form, so the map has no form after the
        // drain, as a tuple has no array after `Tuple::take_last`.
        assert!(copy.repr.is_none());
        pending.clear();
    }

    // A change shares the nodes off its path. The map that drains first takes only the pairs
    // of the nodes that it holds alone, and the last holder of a shared node takes its pairs.
    let mut map = integers(1..=1000);
    let mut copy = map.clone();
    copy.insert(Term::Integer(2000), atom("w"), &test_atom_table())
        .unwrap();
    map.drain_into(&mut pending);
    assert_eq!(pending.len(), 2 * 6);
    drop(map);
    pending.clear();
    copy.drain_into(&mut pending);
    assert_eq!(pending.len(), 2 * 1001);
}

#[test]
fn sorts_pairs_by_the_flatmap_key_order() {
    // ERTS 29.1.1: (id(#{}))#{abs := 1, value := 2} raises {badkey,value},
    // because value (index 745) comes before abs (index 762).
    let mut pairs = vec![
        (Term::static_atom("abs"), Term::Integer(1)),
        (Term::static_atom("value"), Term::Integer(2)),
        (Term::Integer(7), Term::Integer(3)),
    ];
    Map::sort_pairs(&mut pairs, &test_atom_table());
    let sorted: Vec<_> = pairs.iter().map(|(key, _)| key.clone()).collect();
    assert_eq!(
        sorted,
        [
            Term::Integer(7),
            Term::static_atom("value"),
            Term::static_atom("abs")
        ]
    );
    // ERTS 29.1.1: (id(#{a => 1}))#{{value} := 1, {abs} := 2} raises
    // {badkey,{abs}}: in a tuple, atoms compare by name. A tuple comes
    // before a map, and a map before [].
    let mut compound = vec![
        (Term::nil(), Term::Integer(1)),
        (Term::Map(Map::new()), Term::Integer(2)),
        (
            Term::tuple(vec![Term::static_atom("value")]),
            Term::Integer(3),
        ),
        (
            Term::tuple(vec![Term::static_atom("abs")]),
            Term::Integer(4),
        ),
        (Term::static_atom("abs"), Term::Integer(5)),
    ];
    Map::sort_pairs(&mut compound, &test_atom_table());
    let values: Vec<_> = compound.iter().map(|(_, value)| value.clone()).collect();
    assert_eq!(values, [5, 4, 3, 2, 1].map(Term::Integer));
    // ERTS compares two references by their number words (`utils.c:2239-2290`),
    // so the reference 2 comes after the reference 1.
    let mut references = vec![
        (Term::Reference(Reference(2)), Term::Integer(2)),
        (Term::Reference(Reference(1)), Term::Integer(1)),
    ];
    Map::sort_pairs(&mut references, &test_atom_table());
    let values: Vec<_> = references.iter().map(|(_, value)| value.clone()).collect();
    assert_eq!(values, [1, 2].map(Term::Integer));
    // An alias reference compares by its three number words (`src/term/order.rs`),
    // so the alias 2 comes after the alias 1.
    let mut aliases = vec![
        (
            Term::Reference(Reference(2).pid_reference()),
            Term::Integer(2),
        ),
        (
            Term::Reference(Reference(1).pid_reference()),
            Term::Integer(1),
        ),
    ];
    Map::sort_pairs(&mut aliases, &test_atom_table());
    let values: Vec<_> = aliases.iter().map(|(_, value)| value.clone()).collect();
    assert_eq!(values, [1, 2].map(Term::Integer));
}

#[test]
fn builds_the_same_map_from_pairs_in_any_order() {
    let mut forward = integers(1..=40);
    let backward = integers((1..=40).rev());
    assert_eq!(keys(&forward), keys(&backward));
    let mut pending = Vec::new();
    forward.drain_into(&mut pending);
    assert_eq!(pending.len(), 80);
    let mut pending = Vec::new();
    integers([2, 1]).drain_into(&mut pending);
    let v = atom("v");
    assert_eq!(pending, [Term::Integer(1), v.clone(), Term::Integer(2), v]);
}

#[test]
fn splits_a_leaf_into_nodes_until_the_hashes_differ() {
    let leaf = |key: i64| (key as u64, Term::Integer(key), Term::Integer(key));
    // 0x101 and 0x201 select the same slot at levels 0 and 1, and slots 1 and 2 at level 2.
    let child = Node::split(0, leaf(0x201), leaf(0x101), &test_atom_table());
    let (bitmap, children) = node_of(&child);
    assert_eq!((bitmap, children.len()), (0b10, 1));
    let (bitmap, children) = node_of(&children[0]);
    assert_eq!((bitmap, children.len()), (1, 1));
    let (bitmap, children) = node_of(&children[0]);
    assert_eq!(bitmap, 0b110);
    assert!(matches!(
        children,
        [
            Child::Leaf(Term::Integer(0x101), _),
            Child::Leaf(Term::Integer(0x201), _)
        ]
    ));
}

/// Return the bitmap and the children of a node child.
fn node_of(child: &Child) -> (u16, &[Child]) {
    let Child::Node(node) = child else {
        panic!("not a node child")
    };
    (node.bitmap, &node.children)
}

#[test]
fn keeps_keys_with_equal_hashes_in_a_collision_child() {
    let key = |element: i64| Term::list_from(vec![Term::Integer(element)]);
    // Two keys with equal hashes at the last level give a collision child in the term order.
    let child = Node::split(
        HAMT_MAX_LEVEL,
        (0x5, key(2), Term::Integer(2)),
        (0x5, key(1), Term::Integer(1)),
        &test_atom_table(),
    );
    assert!(matches!(
        &child,
        Child::Collision(pairs) if pairs[..] == [(key(1), Term::Integer(1)), (key(2), Term::Integer(2))]
    ));
    // Fabricated hashes: 0x5 for [1], [2], and [3], and 0x15 for [4]. A leaf split takes
    // the real hash of the stored key, so the trie starts from a split of [2] and [3].
    let root = Node {
        bitmap: 1 << 5,
        children: vec![Node::split(
            1,
            (0x5, key(3), Term::Integer(0)),
            (0x5, key(2), Term::Integer(2)),
            &test_atom_table(),
        )],
    };
    let mut hamt = Hamt { root, size: 2 };
    hamt.insert(0x5, key(1), Term::Integer(1), &test_atom_table());
    hamt.insert(0x5, key(3), Term::Integer(3), &test_atom_table());
    hamt.insert(0x15, key(4), Term::Integer(4), &test_atom_table());
    assert_eq!(hamt.size, 4);
    // The only collision child is at level 16, as an ERTS collision node is.
    let mut nodes = vec![(0, &hamt.root)];
    let mut collision_levels = Vec::new();
    while let Some((level, node)) = nodes.pop() {
        for child in &node.children {
            match child {
                Child::Leaf(..) => {}
                Child::Node(inner) => nodes.push((level + 1, inner)),
                Child::Collision(_) => collision_levels.push(level + 1),
            }
        }
    }
    assert_eq!(collision_levels, [HAMT_MAX_LEVEL]);
    let mut map = Map {
        repr: Some(Repr::Hash(hamt).into()),
    };
    let pairs: Vec<_> = map
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    assert_eq!(
        pairs,
        (1..=4)
            .map(|n| (key(n), Term::Integer(n)))
            .collect::<Vec<_>>()
    );
    let mut pending = Vec::new();
    map.drain_into(&mut pending);
    assert_eq!(
        pending,
        [4, 1, 2, 3].map(|n| [key(n), Term::Integer(n)]).concat()
    );

    // ERTS 29.1.1: these two lists have equal map hashes.
    let first = Term::list_from(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
    let second = Term::list_from(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
    assert_eq!(map_hash(&first), map_hash(&second));
    let mut map = integers(1..=32);
    map.insert(first.clone(), Term::Integer(1), &test_atom_table())
        .unwrap();
    map.insert(second.clone(), Term::Integer(2), &test_atom_table())
        .unwrap();
    map.insert(second.clone(), Term::Integer(3), &test_atom_table())
        .unwrap();
    assert_eq!(map.len(), 34);
    map.update(
        vec![(Insert::Set, first.clone(), Term::Integer(4))],
        &test_atom_table(),
    )
    .unwrap();
    assert_eq!(map.get(&first), Some(&Term::Integer(4)));
    assert_eq!(map.get(&second), Some(&Term::Integer(3)));
}

/// Return the map of each key to itself.
fn identity(keys: Vec<Term>) -> Map {
    Map::from_pairs(
        keys.into_iter().map(|key| (key.clone(), key)).collect(),
        &test_atom_table(),
    )
    .unwrap()
}

/// Return the keys of the pairs of a chunk.
fn chunk_keys(map: &Map, path: &Term, direction: Direction) -> (Vec<Term>, Option<i64>) {
    let chunk = map.chunk(path, 1000, direction).unwrap().unwrap();
    let keys = chunk.pairs.iter().map(|(key, _)| (*key).clone()).collect();
    (keys, chunk.next)
}

#[test]
fn takes_a_whole_collision_node_in_a_chunk() {
    // `walks_collision_nodes_and_bignum_paths_as_pinned_erts` in
    // `tests/integration/logger_start.rs` checks these maps and paths against the pinned ERTS.
    // ERTS 29.1.1: these two lists have equal map hashes, so they share a collision node.
    let a = Term::list_from(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
    let b = Term::list_from(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
    let before = [
        19, 213, 340, 110, 91, 164, 399, 139, 361, 212, 41, 114, 100, 29, 347, 365, 171, 140, 13,
        42, 149, 55, 2, 146, 372, 317, 191, 316, 202, 192, 233,
    ];
    let mut keys = list(&before);
    keys.extend([b.clone(), a.clone()]);
    keys.extend(list(&[312, 98, 333]));
    let map = identity(keys.clone());
    // ERTS 29.1.1 in a new process: map_next(0, Map, iterator) gives 33
    // pairs, because the 32nd pair starts the collision node, and the path 369.
    assert_eq!(
        chunk_keys(&map, &Term::Integer(0), Direction::Forward),
        (keys[..33].to_vec(), Some(369))
    );
    // map_next(113, Map, iterator) selects the collision node at depth
    // 16, after the 16 elements of the path digit.
    assert_eq!(
        chunk_keys(&map, &Term::Integer(113), Direction::Forward),
        (keys[31..].to_vec(), None)
    );
    // A path other than 0 has no limit of 32 pairs: -(2^59) reads as
    // 2^59, and 2^59 is a bignum. A leaf above the last element ends the path.
    for path in [-(1 << 59), 1 << 59] {
        assert_eq!(
            chunk_keys(&map, &Term::Integer(path), Direction::Forward),
            (keys.clone(), None)
        );
    }
    // A negative bignum raises badarg.
    assert!(
        map.chunk(&Term::Integer(-(1 << 60)), 1000, Direction::Forward)
            .unwrap()
            .is_none()
    );
    // ERTS 29.1.1: map_next(0, Map, []) gives the last 33 pairs and the path 97.
    let after = [
        312, 98, 333, 7, 398, 285, 90, 343, 38, 325, 326, 284, 281, 56, 153, 61, 99, 113, 383, 201,
        237, 314, 65, 322, 137, 141, 308, 315, 230, 104, 187,
    ];
    let mut keys = list(&[202, 192, 233]);
    keys.extend([b, a]);
    keys.extend(list(&after));
    let map = identity(keys.clone());
    assert_eq!(
        chunk_keys(&map, &Term::Integer(0), Direction::Backward),
        (keys[3..].to_vec(), Some(97))
    );
    // A flatmap needs a small integer path.
    assert!(
        integers([1])
            .chunk(&Term::Integer(1 << 59), 1000, Direction::Backward)
            .unwrap()
            .is_none()
    );
}

#[test]
fn fails_for_a_path_that_erts_returns_as_a_bignum_not_in_normal_form() {
    // `walks_collision_nodes_and_bignum_paths_as_pinned_erts` in
    // `tests/integration/logger_start.rs` checks the paths 2^60 + 18 and 2^60 + 786
    // against the pinned ERTS.
    // The hashes of the last five keys share their low 60 bits in two
    // groups (see becomes_a_hashmap_with_the_erts_order_above_32_keys),
    // so each group is in a node at level 15.
    let deep = [
        329_898_321_527_737_274,
        446_694_106_686_689_078,
        413_866_500_823_436_792,
        -101_714_049_538_469_044,
        -74_925_715_755_171_563,
    ];
    let before = [
        319, 169, 371, 359, 160, 388, 136, 324, 348, 287, 296, 357, 239, 179, 289, 263, 313, 235,
        226, 240, 211, 128, 247, 194, 384, 269, 168, 373, 265,
    ];
    let keys: Vec<_> = list(&before).into_iter().chain(list(&deep)).collect();
    let map = identity(keys.clone());
    // ERTS 29.1.1: map_next(0, Map, iterator) stops in the node at level
    // 15, so the path has 16 elements. ERTS returns 2^60 + 18 as a
    // bignum of two digits, which is not equal to 1152921504606846994.
    let failure = map.chunk(&Term::Integer(0), 1000, Direction::Forward);
    assert_eq!(
        failure.err().unwrap().to_string(),
        "unsupported erts_internal:map_next/3 path of 16 elements: ERTS returns a bignum \
         that is not in normal form or above 2^63 - 1"
    );
    // ERTS 29.1.1: map_next(1152921504606846994, Map, iterator) gives the last two pairs.
    assert_eq!(
        chunk_keys(&map, &Term::Integer((1 << 60) + 18), Direction::Forward),
        (keys[32..].to_vec(), None)
    );
    let after = [
        126, 390, 155, 165, 228, 227, 242, 151, 208, 197, 350, 133, 301, 392, 196, 205, 332, 353,
        177, 225, 142, 223, 275, 200, 377, 145, 107, 108, 184, 255, 272,
    ];
    let keys: Vec<_> = list(&deep).into_iter().chain(list(&after)).collect();
    let map = identity(keys.clone());
    assert!(
        map.chunk(&Term::Integer(0), 1000, Direction::Backward)
            .is_err()
    );
    // ERTS 29.1.1: map_next(1152921504606847762, Map, []) gives the first four pairs.
    assert_eq!(
        chunk_keys(&map, &Term::Integer((1 << 60) + 786), Direction::Backward),
        (keys[..4].to_vec(), None)
    );
    // A path of 15 elements is a bignum of one digit, in normal form only above 2^59 - 1.
    let mut elements = vec![0; 14];
    assert_eq!(encode_path(&elements), Ok(0));
    elements.push(8);
    assert_eq!(encode_path(&elements), Ok(1 << 59));
    *elements.last_mut().unwrap() = 7;
    assert!(encode_path(&elements).is_err());
    elements.extend([1, 1]);
    assert!(encode_path(&elements).is_err());
}

#[test]
fn counts_a_repeated_new_key_one_time_for_the_map_form() {
    // A 31-key flatmap that adds one missing key twice stays a flatmap of
    // 32 keys: the second add replaces the value of the one pair, as an
    // ERTS insert does. It does not become a 33-key hashmap.
    let mut map = integers(1..=31);
    let added = Term::Integer(32);
    map.update(
        vec![
            (Insert::Add, added.clone(), atom("first")),
            (Insert::Add, added.clone(), atom("second")),
        ],
        &test_atom_table(),
    )
    .unwrap();
    assert_eq!(map.len(), 32);
    assert!(!map.is_hashmap());
    assert_eq!(map.get(&added), Some(&atom("second")));
}

#[test]
fn counts_the_steps_of_the_erts_hashmap_merge() {
    let range = |first: i64, last: i64| integers(first..=last);
    // No hashmap merge for two flatmaps, and for an empty flatmap.
    assert_eq!(range(1, 10).merge_steps(&range(5, 20)), None);
    assert_eq!(range(1, 0).merge_steps(&range(1, 40)), None);
    assert_eq!(range(1, 40).merge_steps(&range(1, 0)), None);
    // ERTS 29.1.1: `maps:merge/2` costs one reduction for each 8 steps.
    let costs = [
        ((1, 10), (100, 140), 2),
        ((100, 140), (1, 10), 2),
        ((1, 40), (41, 80), 4),
        ((1, 40), (1, 40), 8),
        ((1, 100), (50, 150), 16),
        ((1, 1000), (500, 1500), 174),
        ((1, 33), (2, 34), 6),
    ];
    for ((first, last), (other_first, other_last), cost) in costs {
        let steps = range(first, last).merge_steps(&range(other_first, other_last));
        assert_eq!(steps.map(|steps| steps / 8), Some(cost), "{first}..{last}");
    }

    // ERTS 29.1.1: these two lists have equal map hashes, so the walk
    // reaches the collision level 16 for them.
    let first = Term::list_from(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
    let second = Term::list_from(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
    let with = |key: &Term| {
        let mut map = range(1, 32);
        map.insert(key.clone(), Term::Integer(1), &test_atom_table())
            .unwrap();
        map
    };
    let costs = [
        (with(&first), with(&second), 10),
        (with(&first), range(1, 33), 6),
        (with(&first), with(&first), 7),
    ];
    for (map, other, cost) in costs {
        assert_eq!(map.merge_steps(&other).map(|steps| steps / 8), Some(cost));
    }

    // ERTS 29.1.1: a node that both maps hold costs no step (`erl_map.c:1608-1612`), and two
    // copies of one map hold one root. A change shares the nodes off its path.
    let map = range(1, 1000);
    let changed = |map: &Map, keys: &[i64]| {
        let mut copy = map.clone();
        for key in keys {
            copy.insert(Term::Integer(*key), atom("x"), &test_atom_table())
                .unwrap();
        }
        copy
    };
    let twice = changed(&changed(&map, &[500]), &[600]);
    let costs = [
        (map.clone(), changed(&map, &[500]), 5),
        (changed(&map, &[500]), map.clone(), 5),
        (map.clone(), changed(&map, &[2000]), 5),
        (map.clone(), changed(&map, &[1, 2, 3, 4, 5, 6, 7, 8]), 23),
        (changed(&map, &[500]), twice, 9),
        (map.clone(), map.clone(), 0),
        (map.clone(), range(1, 1000), 213),
    ];
    for (map, other, cost) in costs {
        assert_eq!(map.merge_steps(&other).map(|steps| steps / 8), Some(cost));
    }
}

/// The number of timed runs of each map operation of
/// [`records_the_map_operation_cost`]. The report gives the minimum of the
/// runs.
const MAP_COST_RUNS: usize = 9;

/// The number of rounds of one timed run of a map operation. The test profile
/// runs fewer rounds, because the check gate only proves that the measurement
/// runs; the report numbers come from the release profile.
const MAP_COST_ROUNDS: usize = if cfg!(debug_assertions) { 20 } else { 200 };

/// The number of keys of the map of [`records_the_map_operation_cost`].
const MAP_COST_KEYS: i64 = 1000;

/// The sum of the keys 1 to [`MAP_COST_KEYS`]. The map of the measurement
/// holds each key as its value, so this sum is the sum of the values of one
/// round of a read or of a walk.
const MAP_COST_KEY_SUM: i64 = MAP_COST_KEYS * (MAP_COST_KEYS + 1) / 2;

/// Print the cost of one call of `body`, in nanosecond, as the minimum of
/// [`MAP_COST_RUNS`] runs. `calls` is the number of calls of one run, and
/// `expected` is the sum that one run must give, so that a body that reads no
/// value and visits no pair cannot report a number.
///
/// # Examples
///
/// ```rust,ignore
/// cost("nothing", 1.0, 0, || 0);
/// ```
fn cost(name: &str, calls: f64, expected: i64, body: impl Fn() -> i64) {
    let mut best = f64::MAX;
    for _ in 0..MAP_COST_RUNS {
        let start = Instant::now();
        // The sum goes to `black_box`, so the compiler cannot remove the work
        // that the timer measures.
        let sum = black_box(body());
        let elapsed = start.elapsed().as_nanos() as f64;
        assert_eq!(sum, expected, "{name} must consume every result");
        best = best.min(elapsed);
    }
    println!("{name}: {:.2} ns for each call", best / calls);
}

/// Record the cost of the map operations that the step 7 gate of
/// `docs/interpreter-dispatch-plan.md` names: a read of a 1000-key hashmap, a
/// put on a copy of it, an update in place of `put_map_exact`, an iteration
/// over it, and a walk of `erts_internal:map_next/3`. Each number is the
/// minimum of [`MAP_COST_RUNS`] runs, in nanosecond for each call.
#[test]
fn records_the_map_operation_cost() {
    // Each key is its own value, so a read and a walk consume an integer and
    // the sum of a round is known.
    let hashmap = Map::from_pairs(
        (1..=MAP_COST_KEYS)
            .map(|key| (Term::Integer(key), Term::Integer(key)))
            .collect(),
        &test_atom_table(),
    )
    .unwrap();
    println!(
        "map operation cost: minimum of {MAP_COST_RUNS} runs of {MAP_COST_ROUNDS} rounds, in nanosecond for each call"
    );
    // The read of the map walks the HAMT for each key and consumes its value.
    cost(
        "get",
        (MAP_COST_ROUNDS as i64 * MAP_COST_KEYS) as f64,
        MAP_COST_ROUNDS as i64 * MAP_COST_KEY_SUM,
        || {
            let mut sum = 0;
            for _ in 0..MAP_COST_ROUNDS {
                for key in 1..=MAP_COST_KEYS {
                    if let Some(Term::Integer(value)) = hashmap.get(&Term::Integer(key)) {
                        sum += black_box(*value);
                    }
                }
            }
            sum
        },
    );
    // The put of a key that is not in the map: `Map::clone` shares the form
    // and the put copies the nodes on the path of the key, as the interpreter
    // does for `put`.
    cost(
        "put",
        MAP_COST_ROUNDS as f64,
        MAP_COST_ROUNDS as i64 * (MAP_COST_KEYS + 1),
        || {
            let mut sum = 0;
            for key in 0..MAP_COST_ROUNDS {
                let mut copy = hashmap.clone();
                copy.insert(
                    Term::Integer(MAP_COST_KEYS + 1 + key as i64),
                    Term::Integer(1),
                    &test_atom_table(),
                )
                .unwrap();
                sum += black_box(copy.len() as i64);
            }
            sum
        },
    );
    // The update in place of `put_map_exact`: `Map::sort_pairs` sorts the one
    // pair and `Map::update` changes the value in the copy that the
    // instruction read. This is the map work of `update_map_exact`
    // (`src/runtime/interpreter.rs`).
    cost(
        "put_map_exact",
        MAP_COST_ROUNDS as f64,
        MAP_COST_ROUNDS as i64 * MAP_COST_KEYS,
        || {
            let mut sum = 0;
            for key in 0..MAP_COST_ROUNDS {
                let mut pairs = vec![(Term::Integer(1 + key as i64), Term::Integer(2))];
                Map::sort_pairs(&mut pairs, &test_atom_table());
                let mut map = hashmap.clone();
                map.update(
                    pairs
                        .into_iter()
                        .map(|(key, value)| (Insert::Set, key, value))
                        .collect(),
                    &test_atom_table(),
                )
                .unwrap();
                sum += black_box(map.len() as i64);
            }
            sum
        },
    );
    // The iteration over the pairs of the map in the map order.
    cost(
        "iterate",
        (MAP_COST_ROUNDS as i64 * MAP_COST_KEYS) as f64,
        MAP_COST_ROUNDS as i64 * MAP_COST_KEY_SUM,
        || {
            let mut sum = 0;
            for _ in 0..MAP_COST_ROUNDS {
                for (_, value) in hashmap.iter() {
                    if let Term::Integer(value) = value {
                        sum += black_box(*value);
                    }
                }
            }
            sum
        },
    );
    // The walk of `erts_internal:map_next/3`: `Map::chunk` from the path 0
    // until no pair remains, with the budget of a full reduction budget (4
    // reductions for each pair), which is the map work of `maps::map_next`
    // (`src/runtime/interpreter/bif/maps.rs:400`).
    cost(
        "map_next",
        (MAP_COST_ROUNDS as i64 * MAP_COST_KEYS) as f64,
        MAP_COST_ROUNDS as i64 * 2 * MAP_COST_KEY_SUM,
        || {
            let mut sum = 0;
            for _ in 0..MAP_COST_ROUNDS {
                let mut path = 0;
                loop {
                    let chunk = hashmap
                        .chunk(&Term::Integer(path), 1000, Direction::Forward)
                        .unwrap()
                        .unwrap();
                    for (key, value) in chunk.pairs {
                        let (Term::Integer(key), Term::Integer(value)) = (key, value) else {
                            unreachable!()
                        };
                        sum += black_box(*key + *value);
                    }
                    match chunk.next {
                        Some(next) => path = next,
                        None => break,
                    }
                }
            }
            sum
        },
    );
}

/// Return the deep copy of a map form: every node of the trie is a new node,
/// and each leaf, collision pair, and pair of a flatmap copies its terms.
fn deep_repr(repr: &Repr) -> Repr {
    match repr {
        Repr::Flat(pairs) => Repr::Flat(pairs.clone()),
        Repr::Hash(hamt) => Repr::Hash(Hamt {
            root: deep_node(&hamt.root),
            size: hamt.size,
        }),
    }
}

/// Return the deep copy of one node of the trie.
fn deep_node(node: &Node) -> Node {
    Node {
        bitmap: node.bitmap,
        children: node.children.iter().map(deep_child).collect(),
    }
}

/// Return the deep copy of one child of a node.
fn deep_child(child: &Child) -> Child {
    match child {
        Child::Leaf(key, value) => Child::Leaf(key.clone(), value.clone()),
        Child::Node(node) => Child::Node(Arc::new(deep_node(node))),
        Child::Collision(pairs) => Child::Collision(pairs.clone()),
    }
}

#[test]
fn prints_the_cost_of_a_map_payload() {
    // The committed measurement of the map payload of the term cost table of
    // `docs/interpreter-dispatch-measurements.md`. The form type is private
    // to this module, so the measurement lives here, and the timing and
    // report helpers live in `term::costs`.
    let map = integers(0..1_000);
    let arc: Arc<Repr> = Arc::clone(map.arc());
    let rc: Rc<Repr> = Rc::new((**map.arc()).clone());
    let costs = super::super::costs::HANDLE_OPERATIONS;
    super::super::costs::clone_drop("arc_clone_drop_map_payload", &arc, costs);
    super::super::costs::clone_drop("rc_clone_drop_map_payload", &rc, costs);
    super::super::costs::deep_copy(
        "deep_copy_map_payload",
        super::super::costs::LARGE_COPY_OPERATIONS,
        || deep_repr(map.arc()),
    );
    super::super::costs::deep_copy(
        "deep_copy_tuple_with_map_1000",
        super::super::costs::LARGE_COPY_OPERATIONS,
        || {
            Term::tuple(vec![Term::Map(Map {
                repr: Some(Arc::new(deep_repr(map.arc()))),
            })])
        },
    );
}

/// The number of keys of one put probe of [`prints_the_cost_of_a_map_put`].
const PUT_COST_SIZES: [i64; 4] = [1, 10, 100, 1000];

/// The number of rounds of one timed run of a put probe.
const PUT_COST_ROUNDS: usize = if cfg!(debug_assertions) { 20 } else { 200 };

/// Return the maps of one put probe: one map for each round, and each map
/// holds the key of the probe. The maps are built once, before the timer.
fn put_maps(pairs: &[(Term, Term)]) -> Vec<Map> {
    (0..PUT_COST_ROUNDS)
        .map(|_| {
            Map::from_pairs(pairs.to_vec(), &test_atom_table()).expect("the probe keys are valid")
        })
        .collect()
}

/// Print the cost of one map put of `body`, in nanosecond for each call, as
/// the minimum of [`MAP_COST_RUNS`] timed runs over the same maps. `body`
/// returns the number of keys of its map, so the compiler cannot remove the
/// work that the timer measures.
///
/// Both paths replace the value of a key that every map already holds, so
/// the timer compares one operation with the same operation. The maps are
/// built before the timer, and the allocation of the maps stays out of it.
fn put_cost(name: &str, pairs: &[(Term, Term)], body: impl Fn(&mut Map) -> i64) -> f64 {
    let mut maps = put_maps(pairs);
    let mut best = f64::MAX;
    for _ in 0..MAP_COST_RUNS {
        let start = Instant::now();
        let sum: i64 = maps.iter_mut().map(|map| black_box(body(map))).sum();
        let elapsed = start.elapsed().as_nanos() as f64;
        black_box(&mut maps);
        black_box(sum);
        best = best.min(elapsed / PUT_COST_ROUNDS as f64);
    }
    println!("{name}: {best:.2} ns for each call");
    best
}

/// Record the cost of a same-register map put on the owned path and on the
/// retained-version path, on a map of 1, 10, 100, and 1000 keys. This is
/// the measurement of the same-register map-take task of
/// `docs/term-performance-design-codex-v2.md`.
///
/// The owned path holds the map in the slot, so the map of the change has
/// one owner and `Map::update` changes the form in place. The
/// retained-version path changes a clone, so the old map keeps the form and
/// the change copies the nodes on the path of the key (`Arc::make_mut`).
/// Each number is the minimum of [`MAP_COST_RUNS`] runs, in nanosecond for
/// each call.
#[test]
fn prints_the_cost_of_a_map_put() {
    println!(
        "map put cost: minimum of {MAP_COST_RUNS} runs of {PUT_COST_ROUNDS} rounds, in nanosecond for each call"
    );
    for keys in PUT_COST_SIZES {
        // Each key is its own value, and the key of the probe is the last
        // key, so both paths replace the value of a key that the map holds.
        let pairs: Vec<(Term, Term)> = (1..=keys)
            .map(|key| (Term::Integer(key), Term::Integer(key)))
            .collect();
        let probe = Term::Integer(keys);
        // The owned path: the map has one owner, so `Map::update` replaces
        // the value in the form of the map.
        put_cost(&format!("put_owned_{keys}"), &pairs, |map| {
            map.update(
                vec![(Insert::Add, probe.clone(), Term::Integer(9))],
                &test_atom_table(),
            )
            .expect("the probe key is valid");
            map.len() as i64
        });
        // The retained-version path: `Map::clone` shares the form with the
        // old map, so the change copies the nodes on the path of the key
        // and the old map keeps its pairs.
        put_cost(&format!("put_retained_{keys}"), &pairs, |map| {
            let mut copy = map.clone();
            copy.update(
                vec![(Insert::Add, probe.clone(), Term::Integer(9))],
                &test_atom_table(),
            )
            .expect("the probe key is valid");
            copy.len() as i64
        });
    }
}

#[test]
fn drains_the_pairs_of_a_map_in_batches() {
    // A flatmap: each pair of the drain costs one unit, and the drain moves
    // one pair for each unit of its budget.
    let mut map = integers([2, 1]);
    let mut drain = Drain::take(&mut map).expect("the only holder takes the form");
    let mut pending = Vec::new();
    assert_eq!(drain.step(&mut pending, 1), 1);
    assert!(!drain.is_empty());
    assert_eq!(drain.step(&mut pending, 1), 1);
    assert!(drain.is_empty());
    assert_eq!(pending.len(), 4);
    // A hashmap: the walk descends its nodes, so it takes more units than the
    // count of its pairs, and it moves every pair.
    let mut map = integers(1..=40);
    let mut drain = Drain::take(&mut map).unwrap();
    let mut pending = Vec::new();
    let mut moved = 0;
    for _ in 0..1000 {
        if drain.is_empty() {
            break;
        }
        moved += drain.step(&mut pending, 1);
    }
    assert!(drain.is_empty());
    assert!(moved > 40, "the walk descends nodes: {moved} units");
    assert_eq!(pending.len(), 80);
    // A second holder keeps the form, so the drain of the first takes nothing.
    let mut map = integers(1..=40);
    let mut copy = map.clone();
    assert!(Drain::take(&mut map).is_none());
    assert!(Drain::take(&mut copy).is_some());
}

#[test]
fn drains_the_pairs_of_a_collision_child_in_batches() {
    // The HAMT of `keeps_keys_with_equal_hashes_in_a_collision_child`, which
    // holds a collision child at the last level.
    let key = |element: i64| Term::list_from(vec![Term::Integer(element)]);
    let root = Node {
        bitmap: 1 << 5,
        children: vec![Node::split(
            1,
            (0x5, key(3), Term::Integer(0)),
            (0x5, key(2), Term::Integer(2)),
            &test_atom_table(),
        )],
    };
    let mut hamt = Hamt { root, size: 2 };
    hamt.insert(0x5, key(1), Term::Integer(1), &test_atom_table());
    hamt.insert(0x5, key(3), Term::Integer(3), &test_atom_table());
    hamt.insert(0x15, key(4), Term::Integer(4), &test_atom_table());
    let mut map = Map {
        repr: Some(Repr::Hash(hamt).into()),
    };
    let mut drain = Drain::take(&mut map).expect("the only holder takes the form");
    assert!(
        drain.allocated_bytes() > 0,
        "the drain holds the nodes of the collision"
    );
    let mut live = crate::term::LiveBytes::new();
    drain.account(&mut live);
    assert!(
        live.bytes() > 0,
        "the live-byte walk counts the collision pairs of the drain"
    );
    let mut pending = Vec::new();
    let mut units = 0;
    for _ in 0..1000 {
        if drain.is_empty() {
            break;
        }
        let moved = drain.step(&mut pending, 1);
        assert!(moved <= 1, "one step of one unit moved {moved} units");
        units += moved;
        // A step that reaches the collision child moves its pairs into the
        // collision list of the drain, and the walk must count them there.
        let mut live = crate::term::LiveBytes::new();
        drain.account(&mut live);
        assert!(
            live.bytes() > 0,
            "the walk counts the drain at each step of the collision walk"
        );
    }
    assert!(drain.is_empty());
    assert_eq!(
        pending.len(),
        8,
        "the drain moves the pairs of the collision"
    );
    // The four pairs of the collision cost one unit each, and the walk steps
    // down a node for each level of the trie.
    assert!(units > 4, "the walk descends nodes: {units} units");
}

#[test]
fn prints_the_state_of_a_drain() {
    // The drain prints the count of the nodes and of the collision pairs that
    // are left.
    let mut map = integers(1..=40);
    let drain = Drain::take(&mut map).expect("the only holder takes the form");
    let text = format!("{drain:?}");
    assert!(text.starts_with("Drain"), "{text}");
    let mut map = integers(1..=40);
    let mut drain = Drain::take(&mut map).unwrap();
    let mut pending = Vec::new();
    while !drain.is_empty() {
        drain.step(&mut pending, 256);
    }
    let text = format!("{drain:?}");
    assert!(text.contains("nodes: 0"), "{text}");
}

#[test]
fn reports_the_bytes_of_the_form_and_the_nodes() {
    let flat = integers([1, 2]);
    assert!(flat.allocated_bytes() > 0);
    let hash = integers(0..40);
    assert!(hash.allocated_bytes() > flat.allocated_bytes());
    assert!(Node::empty().allocated_bytes() >= std::mem::size_of::<Node>());
}

#[test]
fn counts_a_collision_child_in_the_walk() {
    let key = |element: i64| Term::list_from(vec![Term::Integer(element)]);
    let child = Node::split(
        HAMT_MAX_LEVEL,
        (0x5, key(2), Term::Integer(2)),
        (0x5, key(1), Term::Integer(1)),
        &test_atom_table(),
    );
    let root = Node {
        bitmap: 1 << 5,
        children: vec![child],
    };
    let hamt = Hamt { root, size: 2 };
    let map = Map {
        repr: Some(Arc::new(Repr::Hash(hamt))),
    };
    let mut live = crate::term::LiveBytes::new();
    live.term(&Term::Map(map));
    assert!(live.bytes() > 0);
}

#[test]
fn counts_the_terms_of_the_values_of_a_drain() {
    // The walk of a drain queues the keys and the values of its leaves, and
    // it flushes that queue before a caller reads the number. A value that
    // holds a long list therefore gives the bytes of that list.
    let width = 1024;
    let mut direct = crate::term::LiveBytes::new();
    direct.term(&Term::list_from((0..width as i64).map(Term::Integer)));
    let expected = direct.bytes();
    assert!(expected > 0, "a list of cells holds live bytes");
    let mut map = Map::from_pairs(
        vec![(
            Term::Integer(0),
            Term::list_from((0..width as i64).map(Term::Integer)),
        )],
        &test_atom_table(),
    )
    .unwrap();
    let drain = Drain::take(&mut map).expect("the only holder takes the form");
    let mut live = crate::term::LiveBytes::new();
    drain.account(&mut live);
    assert!(
        live.bytes() >= expected,
        "the walk counts the list that the value of the map holds"
    );
}
