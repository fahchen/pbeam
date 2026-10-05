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

/// Run `erlang:map_get/2`: return the value of a key in a map, as ERTS
/// `maps_get_2` does (`erl_map.c:250-269`). A key that the map does not
/// hold raises `error:{badkey, Key}`, and a term that is not a map raises
/// `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("input"), test_atom("cooked"))])?);
/// assert!(matches!(get(&test_atom("input"), &map), BifResult::Value(value) if value == test_atom("cooked")));
/// assert!(matches!(get(&test_atom("ofd"), &map), BifResult::Raise { .. }));
/// assert!(matches!(get(&test_atom("input"), &test_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn get(key: &Term, map: &Term) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    entries
        .get(key)
        .map_or_else(|| error_with("badkey", key), |value| Value(value.clone()))
}

/// Run `maps:merge/2`: return the first map with each pair of the second
/// map put into it, so the values of the second map win, as ERTS
/// `maps_merge_2` does (`erl_map.c:1257-1284`). When the first argument is
/// not a map, raise `error:{badmap, First}`; else when the second argument
/// is not a map, raise `error:{badmap, Second}`.
///
/// # Examples
///
/// ```rust,ignore
/// let first = Term::Map(Map::from_pairs(vec![(test_atom("input"), test_atom("raw"))])?);
/// let second = Term::Map(Map::from_pairs(vec![(test_atom("input"), test_atom("cooked"))])?);
/// assert!(matches!(merge(&first, &second)?, BifResult::Value(merged) if merged == second));
/// assert!(matches!(merge(&test_atom("foo"), &second)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when the merged map gets a key that the map form
/// does not support yet, for example an integer outside the small-integer
/// range in a map of more than 32 keys.
pub(super) fn merge(first: &Term, second: &Term) -> Result<BifResult, BeamError> {
    match (first, second) {
        (Term::Map(first), Term::Map(second)) => {
            let pairs = first.iter().chain(second.iter());
            let pairs = pairs.map(|(key, value)| (key.clone(), value.clone()));
            let merged = Map::from_pairs(pairs.collect())
                .map_err(|key| error(format!("unsupported map key {key:?}")))?;
            Ok(Value(Term::Map(merged)))
        }
        (Term::Map(_), _) => Ok(error_with("badmap", second)),
        _ => Ok(error_with("badmap", first)),
    }
}

/// Run `maps:remove/2`: return the map without the key, as ERTS
/// `maps_remove_2` does (`erl_map.c:1929-1937`). A key that the map does not
/// hold, also a key of a type that a PBeam map cannot hold, returns the map
/// itself. A hashmap that keeps 32 keys becomes a flatmap, as
/// `hashmap_delete` makes it (`erl_map.c:2989-3020`). A term that is not a
/// map raises `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))])?);
/// assert!(matches!(remove(&test_atom("a"), &map), BifResult::Value(Term::Map(empty)) if empty.len() == 0));
/// assert!(matches!(remove(&test_atom("b"), &map), BifResult::Value(same) if same == map));
/// assert!(matches!(remove(&test_atom("a"), &test_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn remove(key: &Term, map: &Term) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    if entries.get(key).is_none() {
        return Value(map.clone());
    }
    let pairs = entries.iter().filter(|(existing, _)| *existing != key);
    let pairs = pairs.map(|(key, value)| (key.clone(), value.clone()));
    // Each key of the rest is a key of the map, so the map form supports it.
    let rest = Map::from_pairs(pairs.collect()).expect("the keys of a map are valid map keys");
    Value(Term::Map(rest))
}

/// Return the `error:{Tag, Term}` exception, as ERTS `BIF_ERROR` with
/// `BADMAP` or `BADKEY` and `fvalue` set to the term.
///
/// # Examples
///
/// ```rust,ignore
/// let BifResult::Raise { reason, .. } = error_with("badmap", &test_atom("foo")) else { panic!() };
/// assert_eq!(reason, Term::Tuple(vec![test_atom("badmap"), test_atom("foo")]));
/// ```
fn error_with(tag: &str, term: &Term) -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::Tuple(vec![Term::static_atom(tag), term.clone()]),
    }
}
