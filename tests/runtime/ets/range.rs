use super::{KeyRange, can_match, cmp_partly_bound, key_range, tag};
use crate::runtime::ets::pattern::MatchSpec;
use crate::term::{
    AtomTable, Binary, Fun, Map, PortId, ProcessId, Reference, Term, test_atom_table,
};
use num_bigint::BigInt;
use std::cmp::Ordering;

fn atom(atoms: &AtomTable, name: &str) -> Term {
    Term::Atom(atoms.put(name).unwrap())
}

fn tuple(elements: Vec<Term>) -> Term {
    Term::tuple(elements)
}

fn spec(atoms: &AtomTable, heads: Vec<Term>) -> MatchSpec {
    let clauses = heads
        .into_iter()
        .map(|head| {
            tuple(vec![
                head,
                Term::nil(),
                Term::list_from(vec![Term::Integer(1)]),
            ])
        })
        .collect::<Vec<_>>();
    MatchSpec::compile(atoms, &Term::list_from(clauses)).unwrap()
}

#[test]
fn finds_the_key_range_as_analyze_pattern() {
    // analyze_pattern and key_boundness (erl_db_tree.c:2826-2851, 3828-3849).
    let atoms = test_atom_table();
    let any = atom(&atoms, "_");
    let var = atom(&atoms, "$1");
    let range = |heads| key_range(&atoms, &spec(&atoms, heads), 1).unwrap();
    assert_eq!(range(vec![]), KeyRange::Impossible);
    assert_eq!(range(vec![atom(&atoms, "foo")]), KeyRange::Impossible);
    assert_eq!(range(vec![tuple(vec![])]), KeyRange::Impossible);
    assert_eq!(range(vec![any.clone()]), KeyRange::Unbound);
    assert_eq!(range(vec![var.clone()]), KeyRange::Unbound);
    assert_eq!(range(vec![tuple(vec![var.clone()])]), KeyRange::Unbound);
    let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), var.clone())], &atoms).unwrap());
    assert_eq!(
        range(vec![tuple(vec![tuple(vec![map, var.clone()])])]),
        KeyRange::Unbound
    );
    let a = atom(&atoms, "a");
    assert_eq!(
        range(vec![tuple(vec![a.clone()])]),
        KeyRange::Bound(a.clone())
    );
    // Two equal keys stay bound; two different keys make a range.
    assert_eq!(
        range(vec![
            tuple(vec![Term::Integer(1)]),
            tuple(vec![Term::Float(1.0)])
        ]),
        KeyRange::Bound(Term::Integer(1))
    );
    let b = atom(&atoms, "b");
    assert_eq!(
        range(vec![
            tuple(vec![b.clone()]),
            tuple(vec![a.clone()]),
            atom(&atoms, "x")
        ]),
        KeyRange::Partial {
            least: a.clone(),
            most: b.clone()
        }
    );
    let partial = tuple(vec![Term::Integer(1), any.clone()]);
    assert_eq!(
        range(vec![tuple(vec![partial.clone()]), tuple(vec![a.clone()])]),
        KeyRange::Partial {
            least: a.clone(),
            most: partial.clone()
        }
    );
    assert_eq!(
        range(vec![tuple(vec![partial.clone()]), any]),
        KeyRange::Unbound
    );
}

#[test]
fn compares_a_partially_bound_key_as_cmp_partly_bound() {
    // cmp_partly_bound (erl_db_tree.c:3853-3932).
    let atoms = test_atom_table();
    let any = atom(&atoms, "_");
    let var = atom(&atoms, "$1");
    let a = atom(&atoms, "a");
    let cmp = |x: &Term, y: &Term| cmp_partly_bound(&atoms, x, y).unwrap();
    assert_eq!(cmp(&any, &a), Ordering::Equal);
    assert_eq!(cmp(&a, &a), Ordering::Equal);
    assert_eq!(cmp(&Term::Integer(1), &a), Ordering::Less);
    let one_any = tuple(vec![Term::Integer(1), any.clone(), Term::Integer(9)]);
    assert_eq!(
        cmp(
            &one_any,
            &tuple(vec![Term::Integer(1), a.clone(), Term::Integer(0)])
        ),
        Ordering::Equal
    );
    assert_eq!(
        cmp(
            &one_any,
            &tuple(vec![Term::Integer(2), a.clone(), a.clone()])
        ),
        Ordering::Less
    );
    assert_eq!(
        cmp(&one_any, &tuple(vec![Term::Integer(1)])),
        Ordering::Greater
    );
    let list = Term::list_from(vec![Term::Integer(1), var.clone()]);
    assert_eq!(
        cmp(
            &list,
            &Term::list_from(vec![Term::Integer(1), a.clone(), a.clone()])
        ),
        Ordering::Equal
    );
    assert_eq!(
        cmp(&list, &Term::list_from(vec![Term::Integer(0)])),
        Ordering::Greater
    );
    assert_eq!(cmp(&list, &a), Ordering::Greater);
    let one = Term::list_from(vec![Term::Integer(1)]);
    assert_eq!(cmp(&one, &one), Ordering::Equal);
    // Two tails that are the same word are equal before the variable check
    // (erl_db_tree.c:3880-3881), so the next element decides.
    let tail = Term::cons(Term::Integer(1), var.clone());
    let pair = tuple(vec![tail.clone(), Term::Integer(5)]);
    assert_eq!(
        cmp(&pair, &tuple(vec![tail, Term::Integer(6)])),
        Ordering::Less
    );
    let error = Term::Reference(Reference(1).pid_reference());
    assert!(
        cmp_partly_bound(
            &atoms,
            &error,
            &Term::Reference(Reference(2).pid_reference())
        )
        .is_err()
    );
}

#[test]
fn orders_the_partially_bound_keys_as_partly_bound_can_match() {
    // partly_bound_can_match_lesser and _greater (erl_db_tree.c:3952-4128).
    let atoms = test_atom_table();
    let any = atom(&atoms, "_");
    let var = atom(&atoms, "$1");
    let a = atom(&atoms, "a");
    let lesser = |x: &Term, y: &Term| can_match(&atoms, x, y, Ordering::Less);
    let greater = |x: &Term, y: &Term| can_match(&atoms, x, y, Ordering::Greater);
    assert!(lesser(&any, &a));
    assert!(!lesser(&any, &var));
    assert!(!lesser(&a, &var));
    assert!(lesser(&Term::Integer(1), &a));
    assert!(!lesser(&a, &a));
    assert!(lesser(&Term::Integer(1), &Term::Integer(2)));
    assert!(greater(&Term::Integer(3), &Term::Integer(2)));
    let short = tuple(vec![a.clone()]);
    let long = tuple(vec![a.clone(), a.clone()]);
    assert!(lesser(&short, &long));
    assert!(!lesser(&long, &short));
    assert!(greater(&long, &short));
    assert!(!greater(&short, &long));
    assert!(lesser(&tuple(vec![any.clone()]), &tuple(vec![a.clone()])));
    assert!(!lesser(&tuple(vec![a.clone()]), &tuple(vec![a.clone()])));
    let list = Term::list_from(vec![Term::Integer(1), var.clone()]);
    assert!(lesser(
        &list,
        &Term::list_from(vec![Term::Integer(1), a.clone()])
    ));
    let tail = Term::cons(Term::Integer(1), var.clone());
    assert!(!greater(&tail, &tail));
    assert!(!lesser(
        &Term::Reference(Reference(1).pid_reference()),
        &Term::Reference(Reference(2).pid_reference())
    ));
}

#[test]
fn classifies_the_terms_as_tag_val_def() {
    let atoms = test_atom_table();
    let tags = [
        Term::Integer(1),
        Term::Integer(1 << 60),
        Term::Float(1.0),
        atom(&atoms, "a"),
        Term::Reference(Reference(1)),
        Term::Fun(Fun::external(
            atoms.put("m").unwrap(),
            atoms.put("f").unwrap(),
            0,
        )),
        Term::Port(PortId(1)),
        Term::Pid(ProcessId(1)),
        Term::tuple(vec![]),
        Term::Nil,
        Term::list_from(vec![Term::Nil]),
        Term::Binary(Binary::from(&[1_u8])),
        Term::Map(Map::from_pairs(vec![], &atoms).unwrap()),
    ]
    .iter()
    .map(tag)
    .collect::<Vec<_>>();
    assert_eq!(tags, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    assert_eq!(tag(&Term::from_bigint(BigInt::from(1) << 70)), 1);
}
