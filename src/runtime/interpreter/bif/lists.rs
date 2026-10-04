//! Bodies of the `lists` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `lists:keyfind/3`: return the first tuple in `list` whose element at
/// the 1-based `position` equals `key`, or `false`. Elements that are not
/// tuples, and tuples with fewer than `position` elements, do not match, as
/// in ERTS `keyfind` (`erl_bif_lists.c`). ERTS compares numbers with `==`;
/// PBeam has only integers, so exact equality gives the same result. A
/// `position` below 1 gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let entry = Term::Tuple(vec![Term::static_atom("ok"), Term::Integer(1)]);
/// let found = keyfind(&Term::static_atom("ok"), 1, &[entry.clone()])?;
/// assert!(matches!(found, BifResult::Value(term) if term == entry));
/// assert!(matches!(keyfind(&Term::Integer(1), 1, &[])?, BifResult::Value(Term::Atom(atom)) if atom.name() == "false"));
/// assert!(keyfind(&Term::Integer(1), 0, &[]).is_err());
/// ```
pub(super) fn keyfind(key: &Term, position: i64, list: &[Term]) -> Result<BifResult, BeamError> {
    // ponytail: ERTS raises badarg; see "BIF arguments" in docs/erts-differences.md.
    let Some(index) = usize::try_from(position)
        .ok()
        .and_then(|position| position.checked_sub(1))
    else {
        return Err(error(format!(
            "unsupported lists:keyfind/3 position {position}: below 1"
        )));
    };
    let found = list.iter().find(
        |element| matches!(element, Term::Tuple(elements) if elements.get(index) == Some(key)),
    );
    Ok(Value(
        found.cloned().unwrap_or_else(|| Term::static_atom("false")),
    ))
}
