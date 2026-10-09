use super::{Child, Direction, HAMT_MAX_LEVEL, Hamt, Map, Node, Repr, encode_path, map_hash};
use crate::term::{AtomTable, Reference, Term, internal_hash, test_atom as atom};

fn integers(keys: impl IntoIterator<Item = i64>) -> Map {
    Map::from_pairs(
        keys.into_iter()
            .map(|key| (Term::Integer(key), atom("v")))
            .collect(),
    )
    .unwrap()
}

fn keys(map: &Map) -> Vec<Term> {
    map.iter().map(|(key, _)| key.clone()).collect()
}

fn list(keys: &[i64]) -> Vec<Term> {
    keys.iter().map(|key| Term::Integer(*key)).collect()
}

#[test]
fn hashes_small_integers_as_erts() {
    assert_eq!(map_hash(&Term::Integer(1)), Some(7938660269479274665));
    assert_eq!(map_hash(&Term::Integer(-1)), Some(7256831767414464289));
    assert!(map_hash(&Term::Integer((1 << 59) - 1)).is_some());
    assert!(map_hash(&Term::Integer(-(1 << 59))).is_some());
    assert_eq!(map_hash(&Term::Integer(1 << 59)), None);
    assert_eq!(map_hash(&Term::Integer(-(1 << 59) - 1)), None);
}

#[test]
fn hashes_atoms_by_their_index_as_erts() {
    // true has index 1: its ERTS word is (1 << 6) | 0xB = 0x4B.
    assert_eq!(
        map_hash(&Term::static_atom("true")),
        Some(14558535449716669319)
    );
    // abs_path has index 26. ERTS 29.1.1: its internal hash.
    assert_eq!(
        map_hash(&Term::static_atom("abs_path")),
        Some(17379220480677090226)
    );
}

#[test]
fn keeps_flatmap_keys_in_order_and_replaces_values() {
    // A fresh table for this test makes z before a, so z has the lower
    // atom index. A table shared with other tests could make a first.
    let mut table = AtomTable::new();
    let mut atom = |name: &str| Term::Atom(table.put(name).unwrap());
    let mut map = Map::from_pairs(vec![
        (atom("z"), atom("z")),
        (atom("a"), atom("old")),
        (Term::Integer(2), atom("v")),
        (Term::Integer(i64::MIN), atom("v")),
        (atom("a"), atom("a")),
    ])
    .unwrap();
    assert_eq!(
        keys(&map),
        [
            Term::Integer(i64::MIN),
            Term::Integer(2),
            atom("z"),
            atom("a")
        ]
    );
    // ERTS 29.1.1: maps:keys(#{true => V, abs_path => V, 1 => V}) is [1,true,abs_path],
    // because true has index 1 and abs_path has index 26.
    let static_atoms = Map::from_pairs(vec![
        (Term::static_atom("abs_path"), atom("v")),
        (Term::static_atom("true"), atom("v")),
        (Term::Integer(1), atom("v")),
    ])
    .unwrap();
    assert_eq!(
        keys(&static_atoms),
        [
            Term::Integer(1),
            Term::static_atom("true"),
            Term::static_atom("abs_path")
        ]
    );
    assert_eq!(map.get(&atom("a")), Some(&atom("a")));
    assert_eq!(map.get(&atom("b")), None);
    assert_eq!(map.get(&Term::Tuple(vec![])), None);
    map.insert(atom("z"), atom("new")).unwrap();
    assert_eq!(map.get(&atom("z")), Some(&atom("new")));
    assert_eq!(map.len(), 4);
    // A key type that the flatmap does not support leaves the map unchanged.
    assert_eq!(
        map.insert(Term::Tuple(vec![]), atom("v")),
        Err(Term::Tuple(vec![]))
    );
    assert_eq!(map.len(), 4);
}

#[test]
fn orders_atoms_in_list_keys_by_name_in_a_flatmap() {
    // A fresh table for this test makes z before a, so z has the lower atom index.
    let mut table = AtomTable::new();
    let mut atom = |name: &str| Term::Atom(table.put(name).unwrap());
    let (z, a, v) = (atom("z"), atom("a"), atom("v"));
    let list = |term: &Term| Term::List(vec![term.clone()]);
    let map = Map::from_pairs(vec![
        (list(&z), v.clone()),
        (a.clone(), v.clone()),
        (list(&a), v.clone()),
        (z.clone(), v),
    ])
    .unwrap();
    // erts_cmp_flatmap_keys: two atoms by index, two lists in the term order.
    assert_eq!(keys(&map), [z.clone(), a.clone(), list(&a), list(&z)]);
    assert_eq!(map.get(&list(&z)), Some(&atom("v")));
}

#[test]
fn rejects_list_keys_with_unsupported_elements() {
    assert_eq!(
        map_hash(&Term::List(vec![])),
        Some(internal_hash(&Term::List(vec![])))
    );
    let unsupported = [
        Term::List(vec![Term::Integer(1), Term::Tuple(vec![])]),
        Term::List(vec![Term::List(vec![Term::Tuple(vec![])])]),
        Term::List(vec![Term::Integer(1 << 59)]),
        // [1|{}], [1|<<>>], and [[1|{}]]: an improper list tail that is not an immediate.
        Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::Tuple(vec![]))),
        Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::Binary(vec![]))),
        Term::List(vec![Term::ImproperList(
            vec![Term::Integer(1)],
            Box::new(Term::Tuple(vec![])),
        )]),
    ];
    for key in unsupported {
        assert_eq!(map_hash(&key), None);
        let mut flat = integers([1]);
        assert_eq!(flat.insert(key.clone(), atom("v")), Err(key.clone()));
        assert_eq!(keys(&flat), list(&[1]));
        let mut hash = integers(1..=33);
        assert_eq!(hash.insert(key.clone(), atom("v")), Err(key.clone()));
        assert_eq!(hash.len(), 33);
        assert_eq!(hash.get(&key), None);
    }
}

#[test]
fn hashes_improper_list_keys_with_their_tail_as_erts() {
    // ERTS 29.1.1: maps:keys of #{[1|2] => v, [1,2|3] => v, [[1|2]|3] => v,
    // [true|false] => v, [256|1] => v, [[1]|2] => v, [[]|-1] => v, 1 => v, ..., 26 => v}.
    let improper = |elements: Vec<Term>, tail| Term::ImproperList(elements, Box::new(tail));
    let int = Term::Integer;
    let one_two = || improper(vec![int(1)], int(2));
    let one_two_three = || improper(vec![int(1), int(2)], int(3));
    let nested = || improper(vec![one_two()], int(3));
    let booleans = || improper(vec![Term::static_atom("true")], Term::static_atom("false"));
    let large = || improper(vec![int(256)], int(1));
    let list_head = || improper(vec![Term::List(vec![int(1)])], int(2));
    let nil_head = || improper(vec![Term::List(vec![])], int(-1));
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
    map.insert(Term::Integer(32), Term::Integer(0)).unwrap();
    assert_eq!(keys(&map), list(&(1..=32).collect::<Vec<_>>()));
    map.insert(Term::Integer(33), atom("v")).unwrap();
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
    map.insert(Term::Integer(33), Term::Integer(1)).unwrap();
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
        413866500823436792,
        -101714049538469044,
        -74925715755171563,
        329898321527737274,
        446694106686689078,
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
fn puts_atom_keys_in_a_hashmap_and_rejects_large_integers() {
    let a = atom("a");
    // The 33rd key turns a flatmap with an atom key into a hashmap.
    let mut map = integers(1..=31);
    map.insert(a.clone(), a.clone()).unwrap();
    map.insert(Term::Integer(33), a.clone()).unwrap();
    assert_eq!(map.len(), 33);
    assert_eq!(map.get(&a), Some(&a));
    assert_eq!(map.get(&atom("b")), None);
    // An atom key goes into a hashmap.
    let mut map = integers(1..=33);
    map.insert(a.clone(), Term::Integer(1)).unwrap();
    map.insert(a.clone(), Term::Integer(2)).unwrap();
    assert_eq!(map.len(), 34);
    assert_eq!(map.get(&a), Some(&Term::Integer(2)));
    // A hashmap has no hash for an integer outside the small-integer range yet.
    assert_eq!(
        map.insert(Term::Integer(1 << 59), a.clone()),
        Err(Term::Integer(1 << 59))
    );
    let mut map = integers(1..=31);
    map.insert(Term::Integer(1 << 59), a.clone()).unwrap();
    assert_eq!(
        map.insert(Term::Integer(33), a),
        Err(Term::Integer(1 << 59))
    );
    assert_eq!(map.len(), 32);
}

#[test]
fn changes_the_value_of_an_existing_key_in_place() {
    let mut flat = integers([1, 2]);
    *flat.get_mut(&Term::Integer(2)).unwrap() = Term::Integer(0);
    assert_eq!(flat.get(&Term::Integer(2)), Some(&Term::Integer(0)));
    assert!(flat.get_mut(&Term::Integer(3)).is_none());
    // Each root slot of 1..64 is used, so a key can be in a node below the root.
    let mut hash = integers(1..=64);
    for key in 1..=64 {
        *hash.get_mut(&Term::Integer(key)).unwrap() = Term::Integer(-key);
    }
    assert!((1..=64).all(|key| hash.get(&Term::Integer(key)) == Some(&Term::Integer(-key))));
    assert!(hash.get_mut(&Term::Integer(65)).is_none());
    assert!(integers(1..=33).get_mut(&Term::Integer(34)).is_none());
    assert!(hash.get_mut(&Term::Integer(1 << 59)).is_none());
    assert!(hash.get_mut(&Term::Tuple(vec![])).is_none());
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
    Map::sort_pairs(&mut pairs).unwrap();
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
        (Term::List(vec![]), Term::Integer(1)),
        (Term::Map(Map::new()), Term::Integer(2)),
        (
            Term::Tuple(vec![Term::static_atom("value")]),
            Term::Integer(3),
        ),
        (
            Term::Tuple(vec![Term::static_atom("abs")]),
            Term::Integer(4),
        ),
        (Term::static_atom("abs"), Term::Integer(5)),
    ];
    Map::sort_pairs(&mut compound).unwrap();
    let values: Vec<_> = compound.iter().map(|(_, value)| value.clone()).collect();
    assert_eq!(values, [5, 4, 3, 2, 1].map(Term::Integer));
    let mut references = vec![
        (Term::Reference(Reference(1)), Term::Integer(1)),
        (Term::Reference(Reference(2)), Term::Integer(2)),
    ];
    assert!(
        Map::sort_pairs(&mut references)
            .unwrap_err()
            .to_string()
            .contains("unsupported term order of two references")
    );
}

#[test]
fn builds_the_same_map_from_pairs_in_any_order() {
    let forward = integers(1..=40);
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
    assert!(matches!(
        Node::split(0, leaf(0x201), leaf(0x101)),
        Child::Node(Node { bitmap: 0b10, children }) if matches!(
        &children[..],
        [Child::Node(Node { bitmap: 1, children })] if matches!(
            &children[..],
            [Child::Node(Node { bitmap: 0b110, children })] if matches!(
                &children[..],
                [Child::Leaf(Term::Integer(0x101), _), Child::Leaf(Term::Integer(0x201), _)]
            )
        )
        )
    ));
}

#[test]
fn keeps_keys_with_equal_hashes_in_a_collision_child() {
    let key = |element: i64| Term::List(vec![Term::Integer(element)]);
    // Two keys with equal hashes at the last level give a collision child in the term order.
    let child = Node::split(
        HAMT_MAX_LEVEL,
        (0x5, key(2), Term::Integer(2)),
        (0x5, key(1), Term::Integer(1)),
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
        )],
    };
    let mut hamt = Hamt { root, size: 2 };
    hamt.insert(0x5, key(1), Term::Integer(1));
    hamt.insert(0x5, key(3), Term::Integer(3));
    hamt.insert(0x15, key(4), Term::Integer(4));
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
    let map = Map {
        repr: Repr::Hash(hamt),
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
    let first = Term::List(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
    let second = Term::List(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
    assert_eq!(map_hash(&first), map_hash(&second));
    let mut map = integers(1..=32);
    map.insert(first.clone(), Term::Integer(1)).unwrap();
    map.insert(second.clone(), Term::Integer(2)).unwrap();
    map.insert(second.clone(), Term::Integer(3)).unwrap();
    assert_eq!(map.len(), 34);
    *map.get_mut(&first).unwrap() = Term::Integer(4);
    assert_eq!(map.get(&first), Some(&Term::Integer(4)));
    assert_eq!(map.get(&second), Some(&Term::Integer(3)));
}

/// Return the map of each key to itself.
fn identity(keys: Vec<Term>) -> Map {
    Map::from_pairs(keys.into_iter().map(|key| (key.clone(), key)).collect()).unwrap()
}

/// Return the keys of the pairs of a chunk.
fn chunk_keys(map: &Map, path: Term, direction: Direction) -> (Vec<Term>, Option<i64>) {
    let chunk = map.chunk(&path, 1000, direction).unwrap().unwrap();
    let keys = chunk.pairs.iter().map(|(key, _)| (*key).clone()).collect();
    (keys, chunk.next)
}

#[test]
fn takes_a_whole_collision_node_in_a_chunk() {
    // `walks_collision_nodes_and_bignum_paths_as_pinned_erts` in
    // `tests/integration/logger_start.rs` checks these maps and paths against the pinned ERTS.
    // ERTS 29.1.1: these two lists have equal map hashes, so they share a collision node.
    let a = Term::List(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
    let b = Term::List(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
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
        chunk_keys(&map, Term::Integer(0), Direction::Forward),
        (keys[..33].to_vec(), Some(369))
    );
    // map_next(113, Map, iterator) selects the collision node at depth
    // 16, after the 16 elements of the path digit.
    assert_eq!(
        chunk_keys(&map, Term::Integer(113), Direction::Forward),
        (keys[31..].to_vec(), None)
    );
    // A path other than 0 has no limit of 32 pairs: -(2^59) reads as
    // 2^59, and 2^59 is a bignum. A leaf above the last element ends the path.
    for path in [-(1 << 59), 1 << 59] {
        assert_eq!(
            chunk_keys(&map, Term::Integer(path), Direction::Forward),
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
        chunk_keys(&map, Term::Integer(0), Direction::Backward),
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
        329898321527737274,
        446694106686689078,
        413866500823436792,
        -101714049538469044,
        -74925715755171563,
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
        chunk_keys(&map, Term::Integer((1 << 60) + 18), Direction::Forward),
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
        chunk_keys(&map, Term::Integer((1 << 60) + 786), Direction::Backward),
        (keys[..4].to_vec(), None)
    );
    // A path of 15 elements is a bignum of one digit, in normal form only above 2^59 - 1.
    let mut elements = vec![0; 14];
    assert_eq!(encode_path(elements.clone()), Ok(0));
    elements.push(8);
    assert_eq!(encode_path(elements.clone()), Ok(1 << 59));
    *elements.last_mut().unwrap() = 7;
    assert!(encode_path(elements.clone()).is_err());
    elements.extend([1, 1]);
    assert!(encode_path(elements).is_err());
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
    let first = Term::List(list(&[246, 157, 203, 77, 19, 254, 19, 93]));
    let second = Term::List(list(&[161, 92, 65, 50, 231, 24, 153, 137]));
    let with = |key: &Term| {
        let mut map = range(1, 32);
        map.insert(key.clone(), Term::Integer(1)).unwrap();
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
}
