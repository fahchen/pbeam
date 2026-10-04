//! Bodies of the `maps` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// Run `maps:keys/1`: return the keys of a map in map order. A term that is
/// not a map gives a runtime error.
///
/// # Examples
///
/// ```rust,ignore
/// let result = keys(&Term::Map(map_of_a_to_1))?;
/// assert!(matches!(result, BifResult::Value(Term::List(keys)) if keys == vec![test_atom("a")]));
/// assert!(keys(&Term::Integer(1)).is_err());
/// ```
pub(super) fn keys(map: &Term) -> Result<BifResult, BeamError> {
    match map {
        Term::Map(map) => Ok(Value(Term::List(
            map.iter().map(|(key, _)| key.clone()).collect(),
        ))),
        // ponytail: ERTS raises error {badmap, Map}; see "Exception raise sites" in docs/erts-differences.md.
        _ => Err(error("unsupported maps:keys/1 argument: not a map")),
    }
}
