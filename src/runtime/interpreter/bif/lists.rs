//! Bodies of the `lists` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `lists:keyfind/3`: return the first tuple in `list` whose element at
/// the 1-based `position` equals `key`, or `false`. Elements that are not
/// tuples, and tuples with fewer than `position` elements, do not match, as
/// in ERTS `keyfind` (`erl_bif_lists.c:1302-1381`). ERTS compares numbers
/// with `==`; PBeam has only integers, so exact equality gives the same
/// result. An improper `list` gives a tuple that comes before its tail, and
/// raises `badarg` when no tuple matches. A `position` that is not a small
/// integer or is below 1, and a `list` that is not a list, raise `badarg`.
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
/// let improper = Term::ImproperList(vec![Term::Integer(1)], Box::new(Term::static_atom("x")));
/// assert!(matches!(keyfind(&Term::Integer(1), &Term::Integer(1), &improper), BifResult::Raise { .. }));
/// ```
pub(super) fn keyfind(key: &Term, position: &Term, list: &Term) -> BifResult {
    let index = position
        .small_integer()
        .and_then(|position| usize::try_from(position).ok())
        .and_then(|position| position.checked_sub(1));
    let (Some(index), Term::List(elements) | Term::ImproperList(elements, _)) = (index, list)
    else {
        return BifResult::badarg();
    };
    let found = elements.iter().find(
        |element| matches!(element, Term::Tuple(elements) if elements.get(index) == Some(key)),
    );
    match (found, list) {
        (Some(found), _) => Value(found.clone()),
        (None, Term::List(_)) => Value(Term::static_atom("false")),
        (None, _) => BifResult::badarg(),
    }
}

/// Run `lists:keymember/3`: return `true` when [`keyfind`] finds a tuple,
/// else the result of [`keyfind`], as ERTS `lists_keymember_3` does
/// (`erl_bif_lists.c`). Thus `false` comes when no tuple matches, and the
/// arguments that [`keyfind`] does not accept raise `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let entry = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// let found = keymember(&Term::static_atom("ok"), &Term::Integer(1), &Term::List(vec![entry]));
/// assert!(matches!(found, BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// let missing = keymember(&Term::Integer(1), &Term::Integer(1), &Term::List(vec![]));
/// assert!(matches!(missing, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(matches!(keymember(&Term::Integer(1), &Term::Integer(0), &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn keymember(key: &Term, position: &Term, list: &Term) -> BifResult {
    match keyfind(key, position, list) {
        Value(Term::Tuple(_)) => Value(Term::static_atom("true")),
        result => result,
    }
}

/// Run `lists:member/2`: return `true` when an element of `list` is
/// exactly equal to `element`, else `false`, as ERTS `lists_member_2`
/// (`erl_bif_lists.c:1108-1141`) does. An improper `list` gives `true` for
/// an element that comes before its tail, and raises `badarg` when no
/// element matches. A `list` that is not a list raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// let improper = Term::ImproperList(vec![Term::Integer(1)], Box::new(x.clone()));
/// assert!(matches!(member(&Term::Integer(1), &improper), BifResult::Value(Term::Atom(atom)) if atom.name() == "true"));
/// assert!(matches!(member(&Term::Integer(2), &Term::List(vec![])), BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(matches!(member(&Term::Integer(2), &improper), BifResult::Raise { .. }));
/// assert!(matches!(member(&Term::Integer(2), &x), BifResult::Raise { .. }));
/// ```
pub(super) fn member(element: &Term, list: &Term) -> BifResult {
    match list {
        Term::List(elements) | Term::ImproperList(elements, _) if elements.contains(element) => {
            Value(Term::static_atom("true"))
        }
        Term::List(_) => Value(Term::static_atom("false")),
        _ => BifResult::badarg(),
    }
}

/// Run `lists:reverse/2`: return the elements of `list` in reverse order,
/// followed by `tail`, as ERTS `lists_reverse_2` (`erl_bif_lists.c:1195-1263`)
/// does. `tail` can be any term, so `lists:reverse([a], x)` gives the
/// improper list `[a|x]`, and an empty `list` gives `tail` itself. A `list`
/// that is not a proper list raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let x = Term::static_atom("x");
/// assert!(matches!(reverse(&Term::List(vec![]), &x), BifResult::Value(term) if term == x));
/// let list = Term::List(vec![Term::Integer(1), Term::Integer(2)]);
/// let reversed = reverse(&list, &Term::List(vec![Term::Integer(3)]));
/// assert!(matches!(reversed, BifResult::Value(Term::List(elements)) if elements == [2, 1, 3].map(Term::Integer)));
/// assert!(matches!(reverse(&list, &x), BifResult::Value(Term::ImproperList(..))));
/// assert!(matches!(reverse(&x, &Term::List(vec![])), BifResult::Raise { .. }));
/// ```
pub(super) fn reverse(list: &Term, tail: &Term) -> BifResult {
    match list {
        Term::List(list) => Value(Term::list_with_tail(
            list.iter().rev().cloned().collect(),
            tail.clone(),
        )),
        _ => BifResult::badarg(),
    }
}
