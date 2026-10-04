//! The ERTS term order, as `erts_cmp_compound` (`utils.c`, OTP 29.1.1)
//! gives it to `is_ge` with arithmetic comparison (`exact = 0`).
//!
//! Terms of different types compare by type, as the `*_DEF` values in
//! `erl_term.h` order them:
//!
//! `number < atom < reference < fun < port < pid < tuple < map < [] < list < bitstring`
//!
//! PBeam has no float, fun, or port term, so a number is an integer and
//! integers compare by value. Terms of one type compare as follows:
//!
//! - atom: by the bytes of the UTF-8 name, as `erts_cmp_atoms` does, not by
//!   the atom index;
//! - pid: by the process counter, which gives the order of creation, as the
//!   ERTS pid data does until the process table uses a slot again;
//! - tuple: by size, then element by element;
//! - map: by size, then by the keys in term order, then by the values in key
//!   order;
//! - list: element by element, and a shorter list is less when it is a
//!   prefix of the other, because `[]` is less than a list;
//! - binary: by the bytes, then by the size.
//!
//! Two different references have no PBeam order: ERTS orders them by a
//! counter of the scheduler thread that made each one, and the thread
//! number (`erts_set_ref_numbers` in `erl_bif_unique.h`).

use std::cmp::Ordering;

use super::Term;
use crate::error::{BeamError, error};

/// One step of a comparison on the work stack.
enum Step<'a> {
    /// Compare two terms.
    Pair(&'a Term, &'a Term),
    /// The result when every step above this one gives `Equal`: the length
    /// order of two lists.
    Then(Ordering),
}

impl Term {
    /// Compare two terms in the ERTS term order with a work stack, so a deep
    /// term does not recurse. Return the result of the first pair that is
    /// not equal, in the order that ERTS compares the pairs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use std::cmp::Ordering;
    ///
    /// assert_eq!(Term::Integer(1).compare(&Term::static_atom("a"))?, Ordering::Less);
    /// let short = Term::List(vec![Term::static_atom("b")]);
    /// let long = Term::List(vec![Term::static_atom("a"), Term::static_atom("z")]);
    /// assert_eq!(short.compare(&long)?, Ordering::Greater);
    /// assert!(Term::Reference(Reference(1)).compare(&Term::Reference(Reference(2))).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// Return a runtime error when the result depends on the order of two
    /// different references.
    pub(crate) fn compare(&self, other: &Term) -> Result<Ordering, BeamError> {
        let mut steps = vec![Step::Pair(self, other)];
        while let Some(step) = steps.pop() {
            let order = match step {
                Step::Then(order) => order,
                Step::Pair(left, right) => compare_pair(left, right, &mut steps)?,
            };
            if order.is_ne() {
                return Ok(order);
            }
        }
        Ok(Ordering::Equal)
    }
}

/// Compare two terms without their nested terms. For two lists, tuples, or
/// maps that the sizes do not order, push the nested pairs onto `steps` and
/// return `Equal`.
///
/// # Examples
///
/// ```rust,ignore
/// let mut steps = Vec::new();
/// let left = Term::Tuple(vec![Term::Integer(1)]);
/// let right = Term::Tuple(vec![Term::Integer(2)]);
/// assert_eq!(compare_pair(&left, &right, &mut steps)?, Ordering::Equal);
/// assert_eq!(steps.len(), 1);
/// ```
fn compare_pair<'a>(
    left: &'a Term,
    right: &'a Term,
    steps: &mut Vec<Step<'a>>,
) -> Result<Ordering, BeamError> {
    let order = match (left, right) {
        (Term::Integer(left), Term::Integer(right)) => left.cmp(right),
        (Term::Atom(left), Term::Atom(right)) => left.name().cmp(right.name()),
        (Term::Binary(left), Term::Binary(right)) => left.cmp(right),
        (Term::Pid(left), Term::Pid(right)) => left.0.cmp(&right.0),
        (Term::Reference(left), Term::Reference(right)) if left == right => Ordering::Equal,
        (Term::Reference(_), Term::Reference(_)) => {
            return Err(error(format!(
                "unsupported term order of two references {left:?} and {right:?}"
            )));
        }
        (Term::Tuple(left), Term::Tuple(right)) if left.len() == right.len() => {
            steps.extend(left.iter().zip(right).rev().map(|(l, r)| Step::Pair(l, r)));
            Ordering::Equal
        }
        (Term::Tuple(left), Term::Tuple(right)) => left.len().cmp(&right.len()),
        (Term::List(left), Term::List(right)) if !left.is_empty() && !right.is_empty() => {
            steps.push(Step::Then(left.len().cmp(&right.len())));
            steps.extend(left.iter().zip(right).rev().map(|(l, r)| Step::Pair(l, r)));
            Ordering::Equal
        }
        (Term::Map(left), Term::Map(right)) if left.len() == right.len() => {
            // ERTS compares all keys first, then the values (`erts_cmp_compound`).
            let (left, right) = (sorted_pairs(left), sorted_pairs(right));
            let pairs = || left.iter().zip(&right).rev();
            steps.extend(pairs().map(|((_, l), (_, r))| Step::Pair(l, r)));
            steps.extend(pairs().map(|((l, _), (r, _))| Step::Pair(l, r)));
            Ordering::Equal
        }
        (Term::Map(left), Term::Map(right)) => left.len().cmp(&right.len()),
        _ => type_rank(left).cmp(&type_rank(right)),
    };
    Ok(order)
}

/// Return the pairs of a map, sorted by the term order of the keys.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Map::from_pairs(vec![(Term::static_atom("b"), Term::Integer(1)), (Term::Integer(2), Term::Integer(3))])?;
/// let keys: Vec<_> = sorted_pairs(&map).into_iter().map(|(key, _)| key.clone()).collect();
/// assert_eq!(keys, [Term::Integer(2), Term::static_atom("b")]);
/// ```
fn sorted_pairs(map: &super::Map) -> Vec<(&Term, &Term)> {
    let mut pairs: Vec<_> = map.iter().collect();
    pairs.sort_unstable_by(|(left, _), (right, _)| {
        left.compare(right)
            .expect("a map key is an integer or an atom")
    });
    pairs
}

/// Return the rank of the type of a term in the ERTS term order. `[]` and a
/// list that is not empty have different ranks, as `NIL_DEF` and `LIST_DEF`
/// do.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(type_rank(&Term::Integer(1)) < type_rank(&Term::static_atom("a")));
/// assert!(type_rank(&Term::List(vec![])) < type_rank(&Term::List(vec![Term::Integer(1)])));
/// ```
fn type_rank(term: &Term) -> u8 {
    match term {
        Term::Integer(_) => 0,
        Term::Atom(_) => 1,
        Term::Reference(_) => 2,
        Term::Pid(_) => 3,
        Term::Tuple(_) => 4,
        Term::Map(_) => 5,
        Term::List(elements) if elements.is_empty() => 6,
        Term::List(_) => 7,
        Term::Binary(_) => 8,
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering::{Equal, Greater, Less};

    use super::super::{AtomTable, Map, ProcessId, Reference, Term, test_atom as atom};

    fn map(pairs: Vec<(Term, Term)>) -> Term {
        Term::Map(Map::from_pairs(pairs).unwrap())
    }

    #[test]
    fn orders_terms_of_different_types_by_type() {
        // ERTS 29.1.1: lists:sort/1 gives
        // [-1,1,ab,#Ref<...>,<0.10.0>,{},#{},[],[a],<<>>] for these terms.
        let sorted = [
            Term::Integer(-1),
            Term::Integer(1),
            atom("ab"),
            Term::Reference(Reference(1)),
            Term::Pid(ProcessId(0)),
            Term::Tuple(vec![]),
            map(vec![]),
            Term::List(vec![]),
            Term::List(vec![atom("a")]),
            Term::Binary(vec![]),
        ];
        for (index, left) in sorted.iter().enumerate() {
            for (other, right) in sorted.iter().enumerate() {
                assert_eq!(left.compare(right).unwrap(), index.cmp(&other));
            }
        }
    }

    #[test]
    fn orders_atoms_by_name_and_not_by_index() {
        // A fresh table gives z the lower index. ERTS 29.1.1: z >= a is true,
        // ab >= abc is false, and 'é' >= z is true.
        let mut table = AtomTable::new();
        let z = Term::Atom(table.put("pbeam_order_z").unwrap());
        let a = Term::Atom(table.put("pbeam_order_a").unwrap());
        assert_eq!(z.compare(&a).unwrap(), Greater);
        assert_eq!(atom("ab").compare(&atom("abc")).unwrap(), Less);
        let accent = Term::Atom(table.put("é").unwrap());
        assert_eq!(accent.compare(&atom("z")).unwrap(), Greater);
        // Map values compare in the term order of the keys: a, then z.
        let first = map(vec![
            (z.clone(), Term::Integer(1)),
            (a.clone(), Term::Integer(2)),
        ]);
        let second = map(vec![(z, Term::Integer(2)), (a, Term::Integer(1))]);
        assert_eq!(first.compare(&second).unwrap(), Greater);
    }

    #[test]
    fn orders_nested_terms_as_pinned_erts() {
        let list = |names: &[&str]| Term::List(names.iter().map(|name| atom(name)).collect());
        let tuple = |names: &[&str]| Term::Tuple(names.iter().map(|name| atom(name)).collect());
        let pairs = [
            (tuple(&["b"]), tuple(&["a", "a"]), Less),
            (tuple(&["a", "b"]), tuple(&["a", "a"]), Greater),
            (list(&["a", "b"]), list(&["a", "c"]), Less),
            (list(&["a", "b"]), list(&["a"]), Greater),
            (list(&["b"]), list(&["a", "z"]), Greater),
            (Term::Binary(vec![1, 2]), Term::Binary(vec![1, 3]), Less),
            (Term::Binary(vec![1, 2]), Term::Binary(vec![1]), Greater),
            (Term::Binary(vec![2]), Term::Binary(vec![1, 9]), Greater),
            (Term::Pid(ProcessId(2)), Term::Pid(ProcessId(1)), Greater),
            (
                map(vec![(atom("a"), Term::Integer(1))]),
                map(vec![]),
                Greater,
            ),
            (
                map(vec![(atom("z"), Term::Integer(1))]),
                map(vec![
                    (atom("a"), Term::Integer(1)),
                    (atom("b"), Term::Integer(1)),
                ]),
                Less,
            ),
            (
                map(vec![(atom("a"), Term::Integer(2))]),
                map(vec![(atom("b"), Term::Integer(1))]),
                Less,
            ),
            (
                map(vec![(Term::Integer(1), atom("x"))]),
                map(vec![(atom("a"), atom("x"))]),
                Less,
            ),
            (
                Term::Tuple(vec![Term::Reference(Reference(1)), list(&["a"])]),
                Term::Tuple(vec![Term::Reference(Reference(1)), list(&["a"])]),
                Equal,
            ),
        ];
        for (left, right, order) in pairs {
            assert_eq!(
                left.compare(&right).unwrap(),
                order,
                "{left:?} and {right:?}"
            );
        }
        let deep =
            |levels| (0..levels).fold(Term::List(vec![]), |inner, _| Term::List(vec![inner]));
        assert_eq!(deep(1_000_000).compare(&deep(999_999)).unwrap(), Greater);
    }

    #[test]
    fn rejects_the_order_of_two_different_references() {
        // ERTS 29.1.1: a reference made later on scheduler 1 is less than one
        // made earlier on scheduler 2, so PBeam cannot give the ERTS order.
        let pair = Term::Tuple(vec![atom("a"), Term::Reference(Reference(1))]);
        let other = Term::Tuple(vec![atom("a"), Term::Reference(Reference(2))]);
        assert_eq!(
            pair.compare(&other).unwrap_err().to_string(),
            "unsupported term order of two references Reference(Reference(1)) and \
             Reference(Reference(2))"
        );
        let earlier = Term::Tuple(vec![atom("a"), Term::Reference(Reference(2))]);
        assert_eq!(
            Term::Tuple(vec![atom("b"), Term::Reference(Reference(1))])
                .compare(&earlier)
                .unwrap(),
            Greater
        );
    }
}
