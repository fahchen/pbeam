//! The key range of a match specification on an `ordered_set` table, as
//! `analyze_pattern` (`erl_db_tree.c:2756-2875`) finds it, and the
//! comparisons of a partially bound key with a key of the table
//! (`erl_db_tree.c:3853-4128`).

use std::cmp::Ordering;

use super::pattern::{MatchSpec, fully_bound, is_skip, variable};
use crate::error::BeamError;
use crate::term::{AtomTable, Term, external_id::ExternalData};

/// The keys that a match specification can match, as the
/// `ms_key_boundness` of `analyze_pattern` gives them
/// (`erl_db_tree.c:220-226`, `2826-2851`).
#[derive(Debug, Clone, PartialEq)]
pub(in crate::runtime) enum KeyRange {
    /// No object can match.
    Impossible,
    /// Only the object of this key can match.
    Bound(Term),
    /// Only an object whose key is in the range from the least to the most
    /// partially bound key can match.
    Partial { least: Term, most: Term },
    /// Each object can match.
    Unbound,
}

/// Return the key range of the specification `spec` on a table with the
/// 1-based key position `keypos`, as `analyze_pattern` does
/// (`erl_db_tree.c:2826-2851`): the least boundness of the heads, and the
/// least and the most key of the heads with a bound or partially bound key.
/// Several different bound keys make a partially bound range.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(key_range(&atoms, &spec, 1)?, KeyRange::Unbound);
/// ```
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(in crate::runtime) fn key_range(
    atoms: &AtomTable,
    spec: &MatchSpec,
    keypos: usize,
) -> Result<KeyRange, BeamError> {
    // The enum order of `ms_key_boundness`: 0 unbound, 1 partially bound,
    // 2 bound, 3 impossible.
    let mut boundness = 3;
    let (mut least, mut most): (Option<&Term>, Option<&Term>) = (None, None);
    for head in spec.heads() {
        let (level, key) = key_boundness(atoms, head, keypos);
        if let Some(key) = key {
            if least.is_none_or(|least| can_match(atoms, key, least, Ordering::Less)) {
                least = Some(key);
            }
            if most.is_none_or(|most| can_match(atoms, key, most, Ordering::Greater)) {
                most = Some(key);
            }
        }
        boundness = boundness.min(level);
    }
    Ok(match (boundness, least, most) {
        (2, Some(least), Some(most)) if least.compare(most, atoms)?.is_eq() => {
            KeyRange::Bound(least.clone())
        }
        (1 | 2, Some(least), Some(most)) => KeyRange::Partial {
            least: least.clone(),
            most: most.clone(),
        },
        (3, ..) => KeyRange::Impossible,
        _ => KeyRange::Unbound,
    })
}

/// Return the boundness of the key of the head `head` and the key when it
/// is bound or partially bound, as `key_boundness` does
/// (`erl_db_tree.c:3828-3849`).
fn key_boundness<'a>(atoms: &AtomTable, head: &'a Term, keypos: usize) -> (u8, Option<&'a Term>) {
    if is_skip(atoms, head) || variable(atoms, head).is_some() {
        return (0, None);
    }
    let key = match head {
        Term::Tuple(elements) => keypos.checked_sub(1).and_then(|index| elements.get(index)),
        _ => None,
    };
    let Some(key) = key else {
        return (3, None);
    };
    if fully_bound(atoms, key) {
        (2, Some(key))
    } else if !is_skip(atoms, key) && variable(atoms, key).is_none() && !has_map(key) {
        (1, Some(key))
    } else {
        (0, None)
    }
}

/// Return `true` when `term` holds a map (`db_has_map`,
/// `erl_db_util.c:3934-3959`).
fn has_map(term: &Term) -> bool {
    let mut stack = vec![term];
    while let Some(term) = stack.pop() {
        match term {
            Term::Map(_) => return true,
            Term::Tuple(elements) => stack.extend(elements.iter()),
            Term::Cons(_) => {
                let (head, tail) = term.cons_cell().expect("a cons term is a cell");
                stack.push(head);
                stack.push(tail);
            }
            _ => {}
        }
    }
    false
}

/// Return `true` when `term` is `'_'` or a variable.
fn is_wild(atoms: &AtomTable, term: &Term) -> bool {
    is_skip(atoms, term) || variable(atoms, term).is_some()
}

/// Return `true` when the two terms are the same immediate, as `is_same`
/// finds two equal words. The keys of a table are copies, so ERTS finds no
/// other shared term in a key.
fn same_immediate(a: &Term, b: &Term) -> bool {
    match (a, b) {
        (Term::Atom(a), Term::Atom(b)) => a == b,
        (Term::Integer(a), Term::Integer(b)) => a == b,
        (Term::Nil, Term::Nil) => true,
        _ => false,
    }
}

/// The work of one pair of a comparison.
enum Step<'a> {
    /// The pair decides the result.
    Stop(Ordering),
    /// The pair is equal; the next pair decides.
    Next,
    /// Compare these pairs, the first one first.
    Pairs(Vec<Pair<'a>>),
}

/// Two terms to compare, and whether they are the tails of two lists. The
/// list loops compare two tails that are the same word as equal before they
/// look for a variable (`erl_db_tree.c:3880-3881`, `4043-4044`).
type Pair<'a> = (&'a Term, &'a Term, bool);

/// Return the pairs of the elements of two cons cells and of their tails, as
/// the list loops of `erl_db_tree.c:3877-3886` and `4038-4050` walk them.
fn cons_pairs<'a>(a: &'a Term, b: &'a Term) -> Vec<Pair<'a>> {
    let (a_head, a_tail) = a.cons_cell().expect("a cons term is a cell");
    let (b_head, b_tail) = b.cons_cell().expect("a cons term is a cell");
    vec![(a_head, b_head, false), (a_tail, b_tail, true)]
}

/// Return the pairs of the elements of two tuples of one size.
fn tuple_pairs<'a>(a: &'a [Term], b: &'a [Term]) -> Vec<Pair<'a>> {
    a.iter().zip(b.iter()).map(|(a, b)| (a, b, false)).collect()
}

/// Compare the partially bound key `pattern` with the key `key`, as
/// `cmp_partly_bound` does (`erl_db_tree.c:3853-3932`): a variable or `'_'`
/// in the pattern ends the comparison as equal, and the other parts compare
/// in the term order. A work stack walks the terms, so a key of any depth
/// gives a result.
///
/// # Examples
///
/// ```rust,ignore
/// let pattern = Term::tuple(vec![Term::Integer(1), atom("$1")]);
/// assert_eq!(cmp_partly_bound(&atoms, &pattern, &key)?, Ordering::Equal);
/// ```
///
/// # Errors
///
/// Return a runtime error when a comparison meets two alias references.
pub(in crate::runtime) fn cmp_partly_bound(
    atoms: &AtomTable,
    pattern: &Term,
    key: &Term,
) -> Result<Ordering, BeamError> {
    let mut stack = vec![(pattern, key, false)];
    while let Some((a, b, tails)) = stack.pop() {
        if tails && same_immediate(a, b) {
            continue;
        }
        if is_wild(atoms, a) {
            return Ok(Ordering::Equal);
        }
        let step = if same_immediate(a, b) {
            Step::Next
        } else {
            match (a, b) {
                (Term::Cons(_), Term::Cons(_)) => Step::Pairs(cons_pairs(a, b)),
                (Term::Tuple(x), Term::Tuple(y)) if x.len() != y.len() => {
                    Step::Stop(x.len().cmp(&y.len()))
                }
                (Term::Tuple(x), Term::Tuple(y)) => Step::Pairs(tuple_pairs(x, y)),
                _ => match a.compare(b, atoms)? {
                    Ordering::Equal => Step::Next,
                    order => Step::Stop(order),
                },
            }
        };
        match step {
            Step::Stop(order) => return Ok(order),
            Step::Next => {}
            Step::Pairs(pairs) => stack.extend(pairs.into_iter().rev()),
        }
    }
    Ok(Ordering::Equal)
}

/// The class of a term as `tag_val_def` gives it, which `not_eq_tags`
/// compares (`erl_term.h`).
fn tag(term: &Term) -> u8 {
    match term {
        // An integer outside the small range is a bignum in ERTS.
        Term::Integer(_) if term.small_integer().is_some() => 0,
        Term::Integer(_) | Term::BigInt(_) => 1,
        Term::Float(_) => 2,
        Term::Atom(_) => 3,
        Term::Reference(_) | Term::Resource(_) => 4,
        Term::Fun(_) => 5,
        Term::Port(_) => 6,
        Term::Pid(_) => 7,
        Term::External(id) => match id.data {
            ExternalData::Pid { .. } => 14,
            ExternalData::Port(_) => 15,
            ExternalData::Reference(_) => 16,
        },
        Term::Tuple(_) => 8,
        Term::Nil => 9,
        Term::Cons(_) => 10,
        Term::Binary(_) | Term::MatchContext(_) => 11,
        Term::Map(_) => 12,
        Term::Record(_) => 13,
    }
}

/// Return whether the partially bound key `a` can match a key below the
/// least key of `b` (`side` is `Less`), as `partly_bound_can_match_lesser`
/// does, or above the most key of `b` (`side` is `Greater`), as
/// `partly_bound_can_match_greater` does (`erl_db_tree.c:3952-4128`). A
/// comparison that meets two alias references gives `false`, as they meet
/// in no other order of PBeam either.
fn can_match(atoms: &AtomTable, a: &Term, b: &Term, side: Ordering) -> bool {
    let mut stack = vec![(a, b, false)];
    while let Some((a, b, tails)) = stack.pop() {
        if tails && same_immediate(a, b) {
            continue;
        }
        if is_wild(atoms, a) {
            return !is_wild(atoms, b);
        }
        if is_wild(atoms, b) {
            return false;
        }
        let compare = |a: &Term, b: &Term| a.compare(b, atoms).is_ok_and(|order| order == side);
        if tag(a) != tag(b) {
            return compare(a, b);
        }
        match (a, b) {
            (Term::Tuple(x), Term::Tuple(y)) if x.len() != y.len() => {
                if x.len().cmp(&y.len()) == side {
                    return true;
                }
            }
            (Term::Tuple(x), Term::Tuple(y)) => stack.extend(tuple_pairs(x, y).into_iter().rev()),
            (Term::Cons(_), Term::Cons(_)) => stack.extend(cons_pairs(a, b).into_iter().rev()),
            _ => match a.compare(b, atoms) {
                Ok(Ordering::Equal) => {}
                order => return order.is_ok_and(|order| order == side),
            },
        }
    }
    false
}

#[cfg(test)]
#[path = "../../../tests/runtime/ets/range.rs"]
mod tests;
