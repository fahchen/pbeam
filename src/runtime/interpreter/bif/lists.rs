//! Bodies of the `lists` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `lists:keyfind/3`: return the first tuple in `list` whose element at
/// the 1-based `position` equals `key`, or `false`. Elements that are not
/// tuples, and tuples with fewer than `position` elements, do not match, as
/// in ERTS `keyfind` (`erl_bif_lists.c`). ERTS compares numbers with `==`;
/// PBeam has only integers, so exact equality gives the same result. A
/// `position` that is not a small integer or is below 1, and a `list` that
/// is not a list, raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let entry = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// let found = keyfind(&Term::static_atom("ok"), &Term::Integer(1), &Term::List(vec![entry.clone()]));
/// assert!(matches!(found, BifResult::Value(term) if term == entry));
/// let missing = keyfind(&Term::Integer(1), &Term::Integer(1), &Term::List(vec![]));
/// assert!(matches!(missing, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(matches!(keyfind(&Term::Integer(1), &Term::Integer(0), &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn keyfind(key: &Term, position: &Term, list: &Term) -> BifResult {
    let index = position
        .small_integer()
        .and_then(|position| usize::try_from(position).ok())
        .and_then(|position| position.checked_sub(1));
    let (Some(index), Term::List(list)) = (index, list) else {
        return BifResult::badarg();
    };
    let found = list.iter().find(
        |element| matches!(element, Term::Tuple(elements) if elements.get(index) == Some(key)),
    );
    Value(found.cloned().unwrap_or_else(|| Term::static_atom("false")))
}
