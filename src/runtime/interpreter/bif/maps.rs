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

/// Run `maps:find/2`: return `{ok, Value}` for a key that the map holds and
/// `error` for another key, as ERTS `maps_find_2` does
/// (`erl_map.c:225-243`). A term that is not a map raises
/// `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))])?);
/// let found = Term::Tuple(vec![test_atom("ok"), Term::Integer(1)]);
/// assert!(matches!(find(&test_atom("a"), &map), BifResult::Value(value) if value == found));
/// assert!(matches!(find(&test_atom("b"), &map), BifResult::Value(value) if value == test_atom("error")));
/// assert!(matches!(find(&test_atom("a"), &test_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn find(key: &Term, map: &Term) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    Value(entries.get(key).map_or_else(
        || Term::static_atom("error"),
        |value| Term::Tuple(vec![Term::static_atom("ok"), value.clone()]),
    ))
}

/// Run `maps:from_list/1`: return the map of the key and value pairs of a
/// list, as ERTS `maps_from_list_1_helper` does (`erl_map.c:321-363`). The
/// BIF checks the full list before it builds the map: a term that is not a
/// proper list, an element that is not a tuple, and a tuple with an arity
/// other than 2 raise `error:badarg`. A later pair of a key wins, and a list
/// of more than 32 pairs with at most 32 keys gives a flatmap.
///
/// # Examples
///
/// ```rust,ignore
/// let pair = |key: i64, value: &str| Term::Tuple(vec![Term::Integer(key), test_atom(value)]);
/// let list = Term::List(vec![pair(2, "b"), pair(1, "a"), pair(2, "c")]);
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a")), (Term::Integer(2), test_atom("c"))])?);
/// assert!(matches!(from_list(&list)?, BifResult::Value(value) if value == map));
/// assert!(matches!(from_list(&test_atom("foo"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when a key is of a type that the map form does
/// not support yet, for example a tuple key.
pub(super) fn from_list(list: &Term) -> Result<BifResult, BeamError> {
    let Term::List(elements) = list else {
        return Ok(BifResult::badarg());
    };
    let pairs = elements.iter().map(|element| match element {
        Term::Tuple(pair) if pair.len() == 2 => Some((pair[0].clone(), pair[1].clone())),
        _ => None,
    });
    let Some(pairs) = pairs.collect::<Option<Vec<_>>>() else {
        return Ok(BifResult::badarg());
    };
    let map =
        Map::from_pairs(pairs).map_err(|key| error(format!("unsupported map key {key:?}")))?;
    Ok(Value(Term::Map(map)))
}

/// Run `erlang:is_map_key/2`: return `true` when the map holds the key and
/// `false` when it does not, as ERTS `maps_is_key_2` does
/// (`erl_map.c:1185-1196`). A term that is not a map raises
/// `error:{badmap, Map}`. In a guard, the `bif` instruction goes to its fail
/// label instead.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))])?);
/// assert!(matches!(is_key(&test_atom("a"), &map), BifResult::Value(value) if value == test_atom("true")));
/// assert!(matches!(is_key(&test_atom("b"), &map), BifResult::Value(value) if value == test_atom("false")));
/// assert!(matches!(is_key(&test_atom("a"), &test_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn is_key(key: &Term, map: &Term) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    Value(Term::static_atom(if entries.get(key).is_some() {
        "true"
    } else {
        "false"
    }))
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
/// `maps_remove_2` and `erts_maps_take` do (`erl_map.c:1929-1937`,
/// `erl_map.c:1943-2034`). A key that the map does not hold, also a key of a
/// type that a PBeam map cannot hold, returns the map itself. A hashmap that
/// keeps 32 keys becomes a flatmap, as `hashmap_delete` makes it
/// (`erl_map.c:2989-3025`). A term that is not a map raises
/// `error:{badmap, Map}`.
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
