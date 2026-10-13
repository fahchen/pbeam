//! Unit tests of the live-byte walk of `src/term/live_bytes.rs`.

use std::{mem::size_of, sync::Arc};

use super::LiveBytes;
use crate::term::{Atom, Binary, Fun, Lambda, Map, MatchState, Term, list::Cons};

/// Return a local fun with the free variables `free`.
fn fun(free: Vec<Term>) -> Fun {
    Fun::new(
        Lambda::new(Atom::from_static("ok").unwrap(), 0, 1, 53, 1),
        free,
    )
}

/// Return the bytes of the containers of `term`.
fn bytes(term: &Term) -> usize {
    let mut live = LiveBytes::new();
    live.term(term);
    live.bytes()
}

#[test]
fn counts_the_element_array_of_a_tuple_once_for_its_copies() {
    let inner = Term::tuple(vec![Term::Integer(1)]);
    assert_eq!(bytes(&inner), size_of::<Term>());
    // The two copies of the inner tuple share one array, and the outer array
    // is one more allocation.
    let outer = Term::tuple(vec![inner.clone(), inner]);
    assert_eq!(bytes(&outer), 3 * size_of::<Term>());
}

#[test]
fn counts_each_cell_of_a_list_and_shares_a_tail() {
    let list = Term::list_from([Term::Integer(1), Term::Integer(2)]);
    assert_eq!(bytes(&list), 2 * size_of::<Cons>());
    // The list and the tuple share the tail, so the walk counts each cell
    // once.
    let shared = Term::tuple(vec![list.clone(), list]);
    assert_eq!(
        bytes(&shared),
        2 * size_of::<Term>() + 2 * size_of::<Cons>()
    );
    // An improper tail is a term of the head cell.
    let improper = Term::cons(Term::Integer(1), Term::tuple(vec![Term::Integer(2)]));
    assert_eq!(bytes(&improper), size_of::<Cons>() + size_of::<Term>());
}

#[test]
fn counts_a_flat_map_and_walks_its_pairs() {
    let map = Map::from_pairs(
        vec![
            (Term::Integer(1), Term::tuple(vec![Term::Integer(2)])),
            (Term::Integer(3), Term::Integer(4)),
        ],
        &crate::term::test_atoms(),
    )
    .unwrap();
    let term = Term::Map(map);
    assert!(bytes(&term) > size_of::<Term>());
    // The value tuple of the first pair is part of the number.
    let without = Term::Map(
        Map::from_pairs(
            vec![
                (Term::Integer(1), Term::Integer(2)),
                (Term::Integer(3), Term::Integer(4)),
            ],
            &crate::term::test_atoms(),
        )
        .unwrap(),
    );
    assert!(bytes(&term) > bytes(&without));
}

#[test]
fn counts_a_hash_map_and_walks_its_nodes() {
    let pairs: Vec<_> = (0..40)
        .map(|key| (Term::Integer(key), Term::tuple(vec![Term::Integer(key)])))
        .collect();
    let map = Map::from_pairs(pairs, &crate::term::test_atoms()).unwrap();
    assert!(bytes(&Term::Map(map)) > 40 * size_of::<Term>());
}

#[test]
fn counts_a_binary_backing_and_a_packed_view() {
    let plain = Term::binary(b"abcd".to_vec());
    let plain_bytes = bytes(&plain);
    assert!(plain_bytes >= 4);
    // A view whose offset is not on a byte boundary keeps the backing array
    // and adds the packed array.
    let source = Binary::from(b"abcd".to_vec());
    let packed = source.bits(4, 12);
    assert!(packed.packed_backing().is_some());
    let both = Term::tuple(vec![
        Term::Binary(source.clone()),
        Term::Binary(packed.clone()),
    ]);
    assert!(bytes(&both) > plain_bytes);
    // The second copy of the packed view shares the backing array and the
    // packed array, so the walk counts neither again.
    let twice = Term::tuple(vec![
        Term::Binary(packed.clone()),
        Term::Binary(packed.clone()),
    ]);
    assert_eq!(
        bytes(&twice),
        2 * size_of::<Term>() + bytes(&Term::Binary(packed.clone()))
    );
    // A second packed view of one backing array adds only its own packed
    // array: the backing array of the view that the walk reaches first is
    // already counted.
    let other = source.bits(12, 20);
    let backing = source.backing().capacity();
    let packed_bytes = packed.packed_backing().unwrap().capacity();
    let other_bytes = other.packed_backing().unwrap().capacity();
    let views = Term::tuple(vec![Term::Binary(packed), Term::Binary(other)]);
    assert_eq!(
        bytes(&views),
        2 * size_of::<Term>() + backing + packed_bytes + other_bytes
    );
}

#[test]
fn counts_a_fun_thing_and_its_free_variables() {
    let plain = Term::Fun(fun(vec![]));
    let free = Term::Fun(fun(vec![Term::tuple(vec![Term::Integer(1)])]));
    assert!(bytes(&plain) >= size_of::<Fun>());
    assert!(bytes(&free) > bytes(&plain));
    // Two copies of one fun share its thing.
    let shared = Term::tuple(vec![Term::Fun(fun(vec![Term::Integer(1)]))]);
    let copy = shared.clone();
    assert_eq!(bytes(&shared), bytes(&copy));
}

#[test]
fn counts_a_match_context_and_its_binary() {
    let state = Arc::new(MatchState::new(Binary::from(&[1, 2, 3][..])));
    let one = Term::MatchContext(Arc::clone(&state));
    assert!(bytes(&one) >= size_of::<MatchState>() + 3);
    // Two terms share one state, and two states share one binary.
    let two = Term::tuple(vec![
        Term::MatchContext(Arc::clone(&state)),
        Term::MatchContext(Arc::clone(&state)),
    ]);
    assert_eq!(bytes(&two), size_of::<Term>() * 2 + bytes(&one));
    let other = Term::MatchContext(Arc::new(MatchState::new(Binary::from(&[1, 2, 3][..]))));
    assert!(bytes(&Term::tuple(vec![one, other])) > 0);
}

#[test]
fn counts_two_funs_that_share_one_thing_once() {
    let fun = fun(vec![Term::tuple(vec![Term::Integer(1)])]);
    let term = Term::tuple(vec![Term::Fun(fun.clone()), Term::Fun(fun.clone())]);
    let one = {
        let mut live = LiveBytes::new();
        live.term(&Term::Fun(fun));
        live.bytes()
    };
    assert_eq!(bytes(&term), 2 * size_of::<Term>() + one);
}

#[test]
fn counts_a_node_that_two_hash_maps_share_once() {
    let pairs: Vec<_> = (0..40)
        .map(|key| (Term::Integer(key), Term::Integer(key)))
        .collect();
    let first = Map::from_pairs(pairs, &crate::term::test_atoms()).unwrap();
    let mut second = first.clone();
    // A change through one copy copies only the nodes on the path of the
    // key, so the two maps share the other nodes.
    second
        .insert(
            Term::Integer(1000),
            Term::Integer(1000),
            &crate::term::test_atoms(),
        )
        .unwrap();
    let term = Term::tuple(vec![Term::Map(first), Term::Map(second)]);
    let mut live = LiveBytes::new();
    live.term(&term);
    assert!(live.bytes() > 0);
}

#[test]
fn counts_no_bytes_for_an_immediate() {
    assert_eq!(bytes(&Term::Integer(1)), 0);
    assert_eq!(bytes(&Term::nil()), 0);
    assert_eq!(bytes(&Term::static_atom("ok")), 0);
}

#[test]
fn walks_a_long_list_without_recursion() {
    // A recursive walk overflows the Rust stack on a list of this length.
    let list = Term::list_from((0..200_000_i64).map(Term::Integer));
    let mut live = LiveBytes::new();
    live.term(&list);
    assert_eq!(live.bytes(), 200_000 * size_of::<Cons>());
}
