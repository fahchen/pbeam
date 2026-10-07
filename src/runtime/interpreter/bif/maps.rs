//! Bodies of the `maps` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::term::Direction;

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

/// Run `erts_internal:map_next/3`: return the next pairs of a map walk, as
/// ERTS `erts_internal_map_next_3` does (`erl_map.c:3637-4135`).
/// `maps:to_list/1` (`maps.erl:525-542`) calls it with a list third
/// argument, and `maps:next/1` (`maps.erl:1094-1115`) with `iterator`.
///
/// - List mode: the pairs as `{K, V}` before the third argument, after
///   `[Path, Map]` when pairs remain.
/// - Iterator mode with an integer path: `{K1, V1, {K2, V2, Next}}`, where
///   `Next` is `[Path | Map]` when pairs remain and `none` when no pair
///   remains.
/// - Iterator mode with a list path: the ordered iterator of
///   `maps:iterator/2` (`maps.erl:1062-1072`). The BIF takes the keys of the
///   list in order, and `Next` is `[Rest | Map]` or `none`.
///
/// ERTS takes as many pairs as the reductions of the process permit. PBeam
/// has no reductions, so it takes the pairs of a full budget
/// (`CONTEXT_REDS`, `erl_vm.h:53`), as a new ERTS process does.
///
/// A term that is not a map, a third argument that is not `iterator` or a
/// list, a path that `Map::chunk` does not accept, a key that the map does
/// not hold, and a list path with a tail that is not `[]` raise
/// `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a"))])?);
/// let pair = Term::Tuple(vec![Term::Integer(1), test_atom("a")]);
/// assert!(matches!(map_next(&Term::Integer(0), &map, &Term::List(vec![]))?, BifResult::Value(Term::List(pairs)) if pairs == [pair]));
/// assert!(matches!(map_next(&Term::Integer(2), &map, &Term::List(vec![]))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of `Map::chunk` for a next path that PBeam
/// cannot hold.
pub(super) fn map_next(path: &Term, map: &Term, acc: &Term) -> Result<BifResult, BeamError> {
    /// The pairs of a hashmap call with a full budget: `FCALLS_ELEMS` is
    /// `CONTEXT_REDS / 4` (`erl_map.c:3727-3746`).
    const HASHMAP_BUDGET: usize = 1000;
    let Term::Map(entries) = map else {
        return Ok(BifResult::badarg());
    };
    let direction = match acc {
        Term::List(_) | Term::ImproperList(..) => Direction::Backward,
        _ if *acc == Term::static_atom("iterator") => Direction::Forward,
        _ => return Ok(BifResult::badarg()),
    };
    match (direction, path) {
        (Direction::Forward, Term::List(keys)) => {
            return Ok(ordered_next(keys, None, entries, map));
        }
        (Direction::Forward, Term::ImproperList(keys, tail)) => {
            return Ok(ordered_next(keys, Some(tail), entries, map));
        }
        _ => {}
    }
    let Some(chunk) = entries.chunk(path, HASHMAP_BUDGET, direction)? else {
        return Ok(BifResult::badarg());
    };
    let pairs = chunk.pairs.into_iter();
    let next = chunk.next.map(Term::Integer);
    Ok(Value(match direction {
        Direction::Forward => {
            let next = next.map_or(Term::static_atom("none"), |path| {
                Term::list_with_tail(vec![path], map.clone())
            });
            iterator_pairs(pairs, next)
        }
        Direction::Backward => {
            let pairs = pairs.map(|(key, value)| Term::Tuple(vec![key.clone(), value.clone()]));
            let list = Term::list_with_tail(pairs.collect(), acc.clone());
            match next {
                Some(path) => Term::list_with_tail(vec![path, map.clone()], list),
                None => list,
            }
        }
    }))
}

/// Run the ordered iterator of `erts_internal:map_next/3`
/// (`erl_map.c:3653-3707`) for a list path of `keys` and the `tail` of an
/// improper list path: take the keys in order, up to
/// `MAX(1, reductions / 32)` keys. With a full budget, this is 125 keys. A
/// key that the map does not hold, and a tail that is not `[]` after the
/// last key, raise `error:badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a"))])?);
/// let Term::Map(entries) = &map else { unreachable!() };
/// let first = Term::Tuple(vec![Term::Integer(1), test_atom("a"), test_atom("none")]);
/// assert!(matches!(ordered_next(&[Term::Integer(1)], None, entries, &map), BifResult::Value(value) if value == first));
/// assert!(matches!(ordered_next(&[Term::Integer(2)], None, entries, &map), BifResult::Raise { .. }));
/// ```
fn ordered_next(keys: &[Term], tail: Option<&Term>, entries: &Map, map: &Term) -> BifResult {
    /// `CONTEXT_REDS / ORDERED_ITER_FACTOR` (`erl_vm.h:53`, `erl_map.c:3654-3660`).
    const ORDERED_BUDGET: usize = 125;
    let (taken, rest) = keys.split_at(keys.len().min(ORDERED_BUDGET));
    let next = match (rest, tail) {
        ([], None) => Term::static_atom("none"),
        ([], Some(_)) => return BifResult::badarg(),
        (rest, tail) => {
            let tail = tail.cloned().unwrap_or(Term::List(Vec::new()));
            let rest = Term::list_with_tail(rest.to_vec(), tail);
            Term::list_with_tail(vec![rest], map.clone())
        }
    };
    let pairs = taken
        .iter()
        .map(|key| entries.get(key).map(|value| (key, value)));
    let Some(pairs) = pairs.collect::<Option<Vec<_>>>() else {
        return BifResult::badarg();
    };
    Value(iterator_pairs(pairs.into_iter(), next))
}

/// Build the iterator result `{K1, V1, {K2, V2, Next}}` of pairs, or `Next`
/// when there is no pair.
///
/// # Examples
///
/// ```rust,ignore
/// let (key, value) = (Term::Integer(1), test_atom("a"));
/// let result = iterator_pairs([(&key, &value)].into_iter(), test_atom("none"));
/// assert_eq!(result, Term::Tuple(vec![key.clone(), value.clone(), test_atom("none")]));
/// ```
fn iterator_pairs<'a>(
    pairs: impl DoubleEndedIterator<Item = (&'a Term, &'a Term)>,
    next: Term,
) -> Term {
    pairs.rev().fold(next, |rest, (key, value)| {
        Term::Tuple(vec![key.clone(), value.clone(), rest])
    })
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
