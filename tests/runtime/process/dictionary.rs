use super::{Dictionary, make_hash};
use crate::term::Term;

/// Return the keys of `dictionary` in the order of `get_keys/0`.
fn keys(dictionary: &Dictionary) -> Vec<Term> {
    dictionary.pairs().map(|(key, _)| key.clone()).collect()
}

#[test]
fn reads_no_value_before_the_first_put() {
    let dictionary = Dictionary::default();
    assert_eq!(dictionary.get(&Term::Integer(1)), None);
    assert_eq!(dictionary.allocated_bytes(), 0);
}

#[test]
fn hashes_a_small_integer_as_its_value_bits_and_an_atom_as_its_index() {
    assert_eq!(make_hash(&Term::Integer(5)), 5);
    assert_eq!(make_hash(&Term::Integer(-1)), (1 << 60) - 1);
    assert_eq!(make_hash(&Term::static_atom("true")), 1);
}

#[test]
fn relinks_the_last_pair_after_an_erase_from_the_middle_of_a_chain() {
    // Keys 1, 17, and 33 share bucket 1 of 8. The tuple values move 17 and
    // then 1 to the front of the chain, so the last pair, 33, is at the end
    // of the chain. The erase of key 2 moves 33 to the first array slot.
    let mut dictionary = Dictionary::default();
    for key in [2, 1, 17, 33] {
        dictionary.put(Term::Integer(key), Term::Integer(0));
    }
    let tuple = Term::tuple(vec![Term::Integer(0)]);
    dictionary.put(Term::Integer(17), tuple.clone());
    dictionary.put(Term::Integer(1), tuple.clone());
    assert_eq!(dictionary.erase(&Term::Integer(2)), Some(Term::Integer(0)));
    assert_eq!(dictionary.get(&Term::Integer(33)), Some(&Term::Integer(0)));
    // get_keys/0 reverses the walk of bucket 1: 1, 17, 33.
    assert_eq!(
        keys(&dictionary),
        vec![Term::Integer(33), Term::Integer(17), Term::Integer(1)]
    );
    // The erase of 1 moves the last pair, 17, from the front of the chain.
    assert_eq!(dictionary.erase(&Term::Integer(1)), Some(tuple));
    assert_eq!(
        keys(&dictionary),
        vec![Term::Integer(33), Term::Integer(17)]
    );
    assert_eq!(dictionary.take_entries().len(), 2);
    assert_eq!(dictionary.get(&Term::Integer(17)), None);
}
