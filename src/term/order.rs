//! The ERTS term order, as `erts_cmp_compound` (`utils.c`, OTP 29.1.1)
//! gives it to `is_lt` and `is_ge` with arithmetic comparison (`exact = 0`).
//!
//! Terms of different types compare by type, as the `*_DEF` values in
//! `erl_term.h` order them:
//!
//! `number < atom < reference < fun < port < pid < tuple < map < [] < list < bitstring`
//!
//! PBeam has no float or port term, so a number is an integer and integers
//! compare by value. Terms of one type compare as follows:
//!
//! - atom: by the bytes of the UTF-8 name, as `erts_cmp_atoms` does, not by
//!   the atom index;
//! - fun: a local fun is less than an external fun. Local funs compare by
//!   the module name, the lambda index, and the old unique value of the fun
//!   entry, then by the number of free variables, then free variable by
//!   free variable (`utils.c:2157-2187`). External funs compare by the
//!   module name, the function name, and the arity (`utils.c:2189-2207`);
//! - pid: by the process counter, which gives the order of creation, as the
//!   ERTS pid data does until the process table uses a slot again;
//! - tuple: by size, then element by element;
//! - map: by size, then by the keys in term order, then by the values in key
//!   order;
//! - list: element by element, then by the tails, where a proper list has
//!   the tail `[]` (`utils.c:1827-1851`). With equal lengths, the two tails
//!   compare. When one list is shorter, its tail compares with the rest of
//!   the other list, which is a list that is not empty. Thus a shorter
//!   proper list is less when it is a prefix of the other, because `[]` is
//!   less than a list, `[1|x] < [1,2]`, and `[1|<<>>] > [1,2]`;
//! - binary: by the bytes, then by the size.
//!
//! Two different references have no PBeam order: ERTS orders them by a
//! counter of the scheduler thread that made each one, and the thread
//! number (`erts_set_ref_numbers` in `erl_bif_unique.h`).

use std::cmp::Ordering;

use super::{Term, match_context::MATCH_CONTEXT_ESCAPE};
use crate::error::{BeamError, error};

/// The rank of `[]` in the ERTS term order (`NIL_DEF`).
const NIL_RANK: u8 = 7;
/// The rank of a list that is not empty in the ERTS term order (`LIST_DEF`).
const LIST_RANK: u8 = 8;

/// One step of a comparison on the work stack.
enum Step<'a> {
    /// Compare two terms.
    Pair(&'a Term, &'a Term),
    /// The result when every step above this one gives `Equal`: the order
    /// of the tails of two lists that the types of the tails give.
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

/// Compare two terms without their nested terms. For two lists, tuples,
/// maps, or funs that the sizes and fun entries do not order, push the
/// nested pairs onto `steps` and return `Equal`. Two lists that are not
/// empty push their element pairs and then the comparison of their tails.
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
    if let (Some(left), Some(right)) = (list_parts(left), list_parts(right)) {
        steps.extend(compare_tails(left, right));
        steps.extend(
            left.0
                .iter()
                .zip(right.0)
                .rev()
                .map(|(l, r)| Step::Pair(l, r)),
        );
        return Ok(Ordering::Equal);
    }
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
        (Term::Map(left), Term::Map(right)) if left.len() == right.len() => {
            // ERTS compares all keys first, then the values (`erts_cmp_compound`).
            let (left, right) = (sorted_pairs(left), sorted_pairs(right));
            let pairs = || left.iter().zip(&right).rev();
            steps.extend(pairs().map(|((_, l), (_, r))| Step::Pair(l, r)));
            steps.extend(pairs().map(|((l, _), (r, _))| Step::Pair(l, r)));
            Ordering::Equal
        }
        (Term::Map(left), Term::Map(right)) => left.len().cmp(&right.len()),
        (Term::Fun(left), Term::Fun(right)) => {
            let order = left
                .target()
                .compare(right.target())
                .then(left.free().len().cmp(&right.free().len()));
            if order.is_eq() {
                let pairs = left.free().iter().zip(right.free()).rev();
                steps.extend(pairs.map(|(l, r)| Step::Pair(l, r)));
            }
            order
        }
        _ => type_rank(left).cmp(&type_rank(right)),
    };
    Ok(order)
}

/// The elements of a list that is not empty, and its tail: `None` for the
/// tail `[]` of a proper list.
type ListParts<'a> = (&'a [Term], Option<&'a Term>);

/// Return the elements and the tail of a list that is not empty, or `None`
/// for each other term.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// let improper = Term::ImproperList(vec![Term::Integer(1)], Box::new(x.clone()));
/// assert_eq!(list_parts(&improper), Some((&[Term::Integer(1)][..], Some(&x))));
/// assert_eq!(list_parts(&Term::List(vec![Term::Integer(1)])), Some((&[Term::Integer(1)][..], None)));
/// assert_eq!(list_parts(&Term::List(vec![])), None);
/// ```
fn list_parts(term: &Term) -> Option<ListParts<'_>> {
    match term {
        Term::List(elements) if !elements.is_empty() => Some((elements, None)),
        Term::ImproperList(elements, tail) => Some((elements, Some(tail))),
        _ => None,
    }
}

/// Return the step that compares two lists after their common elements, as
/// `erts_cmp_compound` does (`utils.c:1827-1851`). With equal lengths, the
/// two tails compare, and a proper list has the tail `[]`. When one list is
/// shorter, its tail compares with the rest of the other list, which is a
/// list that is not empty. A tail is never a list, so only two improper
/// tails need a nested comparison. Return `None` for two proper lists of
/// equal length.
///
/// # Examples
///
/// ```rust,ignore
/// // [1|x] < [1,2]: the tail x is an atom, which is less than a list.
/// let x = Term::static_atom("x");
/// let step = compare_tails((&[Term::Integer(1)], Some(&x)), (&[Term::Integer(1), Term::Integer(2)], None));
/// assert!(matches!(step, Some(Step::Then(Ordering::Less))));
/// ```
fn compare_tails<'a>(left: ListParts<'a>, right: ListParts<'a>) -> Option<Step<'a>> {
    let rank = |tail: Option<&Term>| tail.map_or(NIL_RANK, type_rank);
    let order = match left.0.len().cmp(&right.0.len()) {
        Ordering::Equal => match (left.1, right.1) {
            (None, None) => return None,
            (Some(left), Some(right)) => return Some(Step::Pair(left, right)),
            (left, right) => rank(left).cmp(&rank(right)),
        },
        Ordering::Less => rank(left.1).cmp(&LIST_RANK),
        Ordering::Greater => LIST_RANK.cmp(&rank(right.1)),
    };
    Some(Step::Then(order))
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
            .expect("a map key is a binary, an integer, an atom, or a list")
    });
    pairs
}

/// Return the rank of the type of a term in the ERTS term order. `[]` and a
/// list that is not empty have different ranks, as `NIL_DEF` and `LIST_DEF`
/// do. An improper list has the rank of a list that is not empty.
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
        Term::Fun(_) => 3,
        Term::Pid(_) => 4,
        Term::Tuple(_) => 5,
        Term::Map(_) => 6,
        Term::List(elements) if elements.is_empty() => NIL_RANK,
        Term::List(_) | Term::ImproperList(..) => LIST_RANK,
        Term::Binary(_) => 9,
        Term::MatchContext(_) => unreachable!("{MATCH_CONTEXT_ESCAPE}"),
    }
}

#[cfg(test)]
#[path = "../../tests/term/order.rs"]
mod tests;
