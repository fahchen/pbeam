use super::{Pattern, variable};
use crate::term::{AtomTable, Term, test_atom_table};

fn atom(atoms: &mut AtomTable, name: &str) -> Term {
    Term::Atom(atoms.put(name).unwrap())
}

#[test]
fn reads_the_variable_numbers_as_db_is_variable() {
    let mut atoms = test_atom_table();
    let mut number = |name| {
        let term = atom(&mut atoms, name);
        variable(&atoms, &term)
    };
    assert_eq!(number("$0"), Some(0));
    assert_eq!(number("$12"), Some(12));
    assert_eq!(number("$01"), None);
    assert_eq!(number("$"), None);
    assert_eq!(number("$1a"), None);
    assert_eq!(number("_"), None);
    // The C `int` arithmetic wraps to a negative number, which is no
    // variable (erl_db_util.c:3913-3926).
    assert_eq!(number("$2147483648"), None);
    assert_eq!(variable(&test_atom_table(), &Term::Integer(1)), None);
}

#[test]
fn matches_lists_maps_and_ground_tuples() {
    let mut atoms = test_atom_table();
    let one = Term::Integer(1);
    let variable = atom(&mut atoms, "$1");
    let list = Term::list_with_tail(vec![variable.clone(), one.clone()], atom(&mut atoms, "_"));
    let pattern = Pattern::compile(&atoms, &Term::tuple(vec![list])).unwrap();
    let object = Term::list_from([Term::Integer(5), one.clone(), Term::Integer(7)]);
    assert_eq!(
        pattern.run(&[object]),
        Some(Term::list_from([Term::Integer(5)]))
    );
    assert_eq!(pattern.run(&[Term::list_from([Term::Integer(5)])]), None);
    assert_eq!(pattern.run(&[Term::nil()]), None);
    // A tuple with no variable stays a tuple step at the root, so it has a
    // bound key and its size.
    let ground = Pattern::compile(&atoms, &Term::tuple(vec![one.clone(), Term::nil()])).unwrap();
    assert_eq!(ground.bound_key(1), Some(&one));
    assert!(ground.matches_nothing(3));
    assert_eq!(ground.run(&[one.clone(), Term::nil()]), Some(Term::nil()));
    let nested =
        Pattern::compile(&atoms, &Term::tuple(vec![Term::tuple(vec![one.clone()])])).unwrap();
    assert!(nested.bound_key(1).is_some());
    let map = Term::Map(
        crate::term::Map::from_pairs(vec![(one.clone(), variable.clone())], &atoms).unwrap(),
    );
    let pattern = Pattern::compile(&atoms, &map).unwrap();
    assert!(pattern.matches_nothing(1));
    assert_eq!(pattern.run(std::slice::from_ref(&one)), None);
    let twice = Term::tuple(vec![variable.clone(), variable.clone()]);
    let pattern = Pattern::compile(&atoms, &twice).unwrap();
    assert_eq!(pattern.run(&[one.clone(), Term::Integer(2)]), None);
    assert_eq!(
        pattern.run(&[one.clone(), one.clone()]),
        Some(Term::list_from([one]))
    );
}
