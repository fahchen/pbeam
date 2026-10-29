//! Bodies of the `maps` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::ycf::{YcfTurn, run_units};
use super::*;
use crate::term::{AtomTable, Direction, debug_term, from_list_units};

/// Run `maps:keys/1`: return the keys of a map in map order, as ERTS
/// `maps_keys_1` does (`erl_map.c:1207-1253`). A term that is not a map
/// gives a runtime error.
///
/// ERTS runs the BIF as YCF code with 15 units for each reduction
/// (`erl_map.c:1242`; [`run_units`]). A flatmap uses no unit. A hashmap
/// uses one unit for each key in the loop of `hashmap_keys`
/// (`erl_map.c:2828-2831`). A yield traps to the export
/// `maps:keys_trap/2` with the map and the units done, which continues with
/// no cost in the next turn. ERTS traps to `maps:keys/1` with a magic
/// reference to the YCF state.
///
/// # Examples
///
/// ```rust,ignore
/// let result = keys(&mut process, &Term::Map(map_of_a_to_1), 0)?;
/// assert!(matches!(result, BifResult::Value(keys) if keys == Term::list_from([test_atom("a")])));
/// assert!(matches!(keys(&mut process, &Term::Integer(1), 0)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when the reduction count fails. A term that is not
/// a map raises `error:{badmap, Term}`, as ERTS `maps_keys_1_helper` does
/// (`erl_map.c:1235-1237`).
pub(super) fn keys(
    process: &mut ProcessState,
    map: &Term,
    done: usize,
) -> Result<BifResult, BeamError> {
    /// `iterations_per_red` of `maps_keys_1` (`erl_map.c:1242`).
    const PER_REDUCTION: usize = 15;
    let Term::Map(entries) = map else {
        return Ok(error_with("badmap", map));
    };
    let units = vec![
        1;
        if entries.is_hashmap() {
            entries.len()
        } else {
            0
        }
    ];
    match run_units(process, PER_REDUCTION, &units, done)? {
        YcfTurn::Done(reductions) => {
            process.bump_reductions(reductions);
            Ok(Value(Term::list_from(
                entries.iter().map(|(key, _)| key.clone()),
            )))
        }
        YcfTurn::Yield(done) => Ok(BifResult::TrapBif {
            module: "maps",
            function: "keys_trap",
            arguments: vec![map.clone(), Term::Integer(done as i64)],
        }),
    }
}

/// Run `erlang:map_get/2` and `maps:get/2`: return the value of a key in a map, as ERTS
/// `maps_get_2` does (`erl_map.c:250-269`). A key that the map does not
/// hold raises `error:{badkey, Key}`, and a term that is not a map raises
/// `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("input"), test_atom("cooked"))], &test_atom_table())?);
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

/// Run `erlang:map_size/1`: return the number of pairs of a flatmap or a
/// hashmap, as ERTS `map_size_1` does (`erl_map.c:146-179`). A term that is
/// not a map raises `error:{badmap, Term}`. The pinned compiler emits the
/// guard BIF as `gc_bif`. In a guard, the instruction goes to its fail label
/// instead, as `emit_bif_map_size` does
/// (`jit/arm/instr_guard_bifs.cpp:947-991`).
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?);
/// assert!(matches!(size(&map), BifResult::Value(Term::Integer(1))));
/// assert!(matches!(size(&test_atom("foo")), BifResult::Raise { .. }));
/// ```
pub(super) fn size(map: &Term) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    Value(Term::Integer(entries.len() as i64))
}

/// Run `maps:find/2`: return `{ok, Value}` for a key that the map holds and
/// `error` for another key, as ERTS `maps_find_2` does
/// (`erl_map.c:225-243`). A term that is not a map raises
/// `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?);
/// let found = Term::tuple(vec![test_atom("ok"), Term::Integer(1)]);
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
        |value| Term::tuple(vec![Term::static_atom("ok"), value.clone()]),
    ))
}

/// Run `maps:put/3`: return the map with the key set to the value, as ERTS
/// `maps_put_3` does (`erl_map.c:1900-1906`). `erts_maps_put`
/// (`erl_map.c:2105-2200`) replaces the value of an equal key, puts a new
/// key of a flatmap in the flatmap key order, and makes a hashmap from a
/// flatmap that gets its 33rd key. A term that is not a map raises
/// `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::new());
/// let key = Term::tuple(vec![Term::Integer(1), Term::Integer(2)]);
/// let put_map = Term::Map(Map::from_pairs(vec![(key.clone(), test_atom("a"))], &test_atom_table())?);
/// assert!(matches!(put(&key, &test_atom("a"), &map, &test_atom_table())?, BifResult::Value(value) if value == put_map));
/// assert!(matches!(put(&key, &test_atom("a"), &test_atom("foo"), &test_atom_table())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when the key is of a type that the map form does
/// not support yet, for example a map key.
pub(super) fn put(
    key: &Term,
    value: &Term,
    map: &Term,
    atoms: &AtomTable,
) -> Result<BifResult, BeamError> {
    let mut map = map.clone();
    let Term::Map(entries) = &mut map else {
        return Ok(error_with("badmap", &map));
    };
    entries
        .insert(key.clone(), value.clone(), atoms)
        .map_err(|key| error(format!("unsupported map key {}", debug_term(&key, atoms))))?;
    Ok(Value(map))
}

/// Run `maps:from_list/1`: return the map of the key and value pairs of a
/// list, as ERTS `maps_from_list_1_helper` does (`erl_map.c:321-363`). The
/// BIF checks the full list before it builds the map: a term that is not a
/// proper list, an element that is not a tuple, and a tuple with an arity
/// other than 2 raise `error:badarg`. A later pair of a key wins, and a list
/// of more than 32 pairs with at most 32 keys gives a flatmap.
///
/// ERTS runs the BIF as YCF code with 40 units for each reduction
/// (`erl_map.c:605-618`; [`run_units`]). The check uses one unit for each
/// element up to the first element that is not a pair (`erl_map.c:332`). A
/// list of at most 32 pairs then builds a flatmap with no unit. A longer
/// list adds the units of the hashmap build ([`from_list_units`]). A yield
/// traps to the export `maps:from_list_trap/2` with the list and the units
/// done, which continues with no cost in the next turn. ERTS traps to
/// `maps:from_list/1` with a magic reference to the YCF state. A `badarg`
/// bumps nothing in its last call (see [`run_units`]). A bad element or
/// tail that the check finds after a yield gives a runtime error: ERTS
/// returns from `BIF_ERROR` before YCF clears its trap state, so the
/// driver traps again from the last yield, forever (`utils.c:3757-3773`).
///
/// # Examples
///
/// ```rust,ignore
/// let pair = |key: i64, value: &str| Term::tuple(vec![Term::Integer(key), test_atom(value)]);
/// let list = Term::list_from(vec![pair(2, "b"), pair(1, "a"), pair(2, "c")]);
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a")), (Term::Integer(2), test_atom("c"))], &test_atom_table())?);
/// assert!(matches!(from_list(&mut process, &list, 0, &test_atom_table())?, BifResult::Value(value) if value == map));
/// assert!(matches!(from_list(&mut process, &test_atom("foo"), 0, &test_atom_table())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when a key is of a type that the map form does
/// not support yet, for example a map key.
pub(super) fn from_list(
    process: &mut ProcessState,
    list: &Term,
    done: usize,
    atoms: &AtomTable,
) -> Result<BifResult, BeamError> {
    /// `iterations_per_red` of `maps_from_list_1` (`erl_map.c:606`).
    const PER_REDUCTION: usize = 40;
    /// `MAP_SMALL_MAP_LIMIT` in a release build (`erl_map.h:93-97`).
    const SMALL_MAP_LIMIT: usize = 32;
    let Some((elements, tail)) = list.list_cells() else {
        return Ok(BifResult::badarg());
    };
    let elements: Vec<&Term> = elements.collect();
    let proper = tail.is_none();
    fn pair(element: &Term) -> Option<(&Term, &Term)> {
        match element {
            Term::Tuple(pair) if pair.len() == 2 => Some((&pair[0], &pair[1])),
            _ => None,
        }
    }
    let pairs = elements
        .iter()
        .map(|element| pair(element))
        .collect::<Option<Vec<_>>>();
    let checked = elements
        .iter()
        .position(|element| pair(element).is_none())
        .map_or(elements.len(), |position| position + 1);
    let mut units = vec![1; checked];
    let pairs = pairs.filter(|_| proper);
    if let Some(pairs) = &pairs
        && pairs.len() > SMALL_MAP_LIMIT
    {
        let keys: Vec<&Term> = pairs.iter().map(|(key, _)| *key).collect();
        units.extend(from_list_units(&keys, atoms)?);
    }
    match run_units(process, PER_REDUCTION, &units, done)? {
        YcfTurn::Yield(done) => Ok(BifResult::TrapBif {
            module: "maps",
            function: "from_list_trap",
            arguments: vec![list.clone(), Term::Integer(done as i64)],
        }),
        YcfTurn::Done(_) if pairs.is_none() && done > 0 => Err(error(
            "unsupported maps:from_list/1 call: maps:from_list/1 finds a bad element after a trap, \
             and ERTS then traps again forever (row \"Scheduling\" in docs/erts-differences.md)",
        )),
        YcfTurn::Done(reductions) => {
            let Some(pairs) = pairs else {
                return Ok(BifResult::badarg());
            };
            process.bump_reductions(reductions);
            let pairs = pairs
                .into_iter()
                .map(|(key, value)| (key.clone(), value.clone()));
            let map = Map::from_pairs(pairs.collect(), atoms)
                .map_err(|key| error(format!("unsupported map key {}", debug_term(&key, atoms))))?;
            Ok(Value(Term::Map(map)))
        }
    }
}

/// Run `maps:from_keys(Keys, Value)`: return the map of each key of a list
/// to `Value`, as ERTS `maps_from_keys_2_helper` does (`erl_map.c:279-311`).
/// A term that is not a proper list raises `error:badarg`. A key that the
/// list has more than one time gives one pair, and a list of more than 32
/// keys with at most 32 distinct keys gives a flatmap.
///
/// ERTS runs the BIF as YCF code with 40 units for each reduction
/// (`erl_map.c:629-642`; [`run_units`]), as for [`from_list`]: the size walk
/// uses one unit for each element (`erl_map.c:287-291`). A list of at most
/// 32 keys then builds a flatmap with no unit. A longer list adds the units
/// of the hashmap build, which `maps:from_list/1` shares
/// (`hashmap_from_validated_list`, `erl_map.c:482-560`;
/// [`from_list_units`]). A yield traps to the export `maps:from_keys_trap/3`
/// with the list, the value, and the units done. ERTS traps to
/// `maps:from_keys/2` with a magic reference to the YCF state. An improper
/// tail that the walk finds after a yield gives the runtime error of
/// [`from_list`].
///
/// # Examples
///
/// ```rust,ignore
/// let keys = Term::list_from(vec![Term::Integer(2), Term::Integer(1), Term::Integer(2)]);
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), Term::nil()), (Term::Integer(2), Term::nil())], &AtomTable::new())?);
/// assert!(matches!(from_keys(&mut process, &keys, &Term::nil(), 0, &test_atom_table())?, BifResult::Value(value) if value == map));
/// assert!(matches!(from_keys(&mut process, &test_atom("foo"), &Term::nil(), 0, &test_atom_table())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when a key is of a type that the map form does
/// not support yet, for example a map key.
pub(super) fn from_keys(
    process: &mut ProcessState,
    keys: &Term,
    value: &Term,
    done: usize,
    atoms: &AtomTable,
) -> Result<BifResult, BeamError> {
    /// `iterations_per_red` of `maps_from_keys_2` (`erl_map.c:630`).
    const PER_REDUCTION: usize = 40;
    /// `MAP_SMALL_MAP_LIMIT` in a release build (`erl_map.h:93-97`).
    const SMALL_MAP_LIMIT: usize = 32;
    let Some((elements, tail)) = keys.list_cells() else {
        return Ok(BifResult::badarg());
    };
    let elements: Vec<&Term> = elements.collect();
    let proper = tail.is_none();
    let mut units = vec![1; elements.len()];
    if proper && elements.len() > SMALL_MAP_LIMIT {
        units.extend(from_list_units(&elements, atoms)?);
    }
    match run_units(process, PER_REDUCTION, &units, done)? {
        YcfTurn::Yield(done) => Ok(BifResult::TrapBif {
            module: "maps",
            function: "from_keys_trap",
            arguments: vec![keys.clone(), value.clone(), Term::Integer(done as i64)],
        }),
        YcfTurn::Done(_) if !proper && done > 0 => Err(error(
            "unsupported maps:from_keys/2 call: maps:from_keys/2 finds an improper tail after a \
             trap, and ERTS then traps again forever (row \"Scheduling\" in docs/erts-differences.md)",
        )),
        YcfTurn::Done(_) if !proper => Ok(BifResult::badarg()),
        YcfTurn::Done(reductions) => {
            process.bump_reductions(reductions);
            let pairs = elements.into_iter().map(|key| (key.clone(), value.clone()));
            let map = Map::from_pairs(pairs.collect(), atoms)
                .map_err(|key| error(format!("unsupported map key {}", debug_term(&key, atoms))))?;
            Ok(Value(Term::Map(map)))
        }
    }
}

/// Run `erlang:is_map_key/2` and `maps:is_key/2`: return `true` when the map holds the key and
/// `false` when it does not, as ERTS `maps_is_key_2` does
/// (`erl_map.c:1185-1196`). A term that is not a map raises
/// `error:{badmap, Map}`. In a guard, the `bif` instruction goes to its fail
/// label instead.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?);
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
///   list in order, and `Next` is `[Rest | Map]` or `none`. See
///   [`ordered_next`].
///
/// A flatmap gives all its pairs with no cost. A hashmap call takes
/// `FCALLS_ELEMS`, the budget divided by 4, but at least 1 pair
/// (`erl_map.c:3783-3792`), and [`Map::chunk`] applies the other limits. The
/// call then subtracts 4 for each pair that it counts from the budget, with
/// no lower limit (`erl_map.c:4126`), so the budget can become negative.
/// `maps.erl` calls the BIF only before a call or a return, which then
/// switch out the process.
///
/// A term that is not a map, a third argument that is not `iterator` or a
/// list, a path that `Map::chunk` does not accept, a key that the map does
/// not hold, and a list path with a tail that is not `[]` raise
/// `error:badarg` with no cost.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a"))], &test_atom_table())?);
/// let pair = Term::tuple(vec![Term::Integer(1), test_atom("a")]);
/// assert!(matches!(map_next(&mut process, &Term::Integer(0), &map, &Term::nil())?, BifResult::Value(pairs) if pairs == Term::list_from([pair])));
/// assert!(matches!(map_next(&mut process, &Term::Integer(2), &map, &Term::nil())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return the runtime error of `Map::chunk` for a next path that PBeam
/// cannot hold.
pub(super) fn map_next(
    process: &mut ProcessState,
    path: &Term,
    map: &Term,
    acc: &Term,
) -> Result<BifResult, BeamError> {
    /// The reductions of each pair of a hashmap call (`FCALLS_ELEMS` and
    /// `erl_map.c:4126`).
    const PAIR_REDUCTIONS: usize = 4;
    let Term::Map(entries) = map else {
        return Ok(BifResult::badarg());
    };
    let direction = match acc {
        _ if acc.is_list() => Direction::Backward,
        _ if *acc == Term::static_atom("iterator") => Direction::Forward,
        _ => return Ok(BifResult::badarg()),
    };
    if direction == Direction::Forward && path.is_list() {
        return ordered_next(process, path, entries, map);
    }
    // `MAX(fcalls / 4, 1)` of the signed budget (`erl_map.c:3786-3790`): a
    // negative budget takes 1 pair, as a budget of 0 does.
    let budget = (process.bif_reductions_or_zero() / PAIR_REDUCTIONS).max(1);
    let Some(chunk) = entries.chunk(path, budget, direction)? else {
        return Ok(BifResult::badarg());
    };
    process.fcalls -= (PAIR_REDUCTIONS * chunk.counted) as i32;
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
            let pairs = pairs.map(|(key, value)| Term::tuple(vec![key.clone(), value.clone()]));
            let list = Term::list_with_tail(pairs.collect(), acc.clone());
            match next {
                Some(path) => Term::list_with_tail(vec![path, map.clone()], list),
                None => list,
            }
        }
    }))
}

/// Run the ordered iterator of `erts_internal:map_next/3`
/// (`erl_map.c:3655-3707`) for a list path of keys, proper or improper:
/// take the keys in order, up to `MAX(1, reductions /
/// ORDERED_ITER_FACTOR)` keys, where the factor is 32. When the path ends
/// in the chunk, the call costs 32 for each key. When keys remain, it uses
/// all reductions. A key that the map does not hold, and a tail that is not
/// `[]` after the last key, raise `error:badarg` with no cost.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a"))], &test_atom_table())?);
/// let Term::Map(entries) = &map else { unreachable!() };
/// process.fcalls = 40;
/// let first = Term::tuple(vec![Term::Integer(1), test_atom("a"), test_atom("none")]);
/// let path = |key| Term::list_from([Term::Integer(key)]);
/// assert!(matches!(ordered_next(&mut process, &path(1), entries, &map)?, BifResult::Value(value) if value == first));
/// assert_eq!(process.fcalls, 8);
/// assert!(matches!(ordered_next(&mut process, &path(2), entries, &map)?, BifResult::Raise { .. }));
/// ```
fn ordered_next(
    process: &mut ProcessState,
    path: &Term,
    entries: &Map,
    map: &Term,
) -> Result<BifResult, BeamError> {
    /// `ORDERED_ITER_FACTOR` in a release build (`erl_map.c:3657-3661`).
    const ORDERED_ITER_FACTOR: usize = 32;
    // `MAX(1, fcalls / 32)` of the signed budget (`erl_map.c:3662`): a
    // negative budget takes 1 key, as a budget of 0 does.
    let budget = (process.bif_reductions_or_zero() / ORDERED_ITER_FACTOR).max(1);
    let mut keys = path.list_iter().expect("an ordered map path is a list");
    let taken: Vec<&Term> = keys.by_ref().take(budget).collect();
    let rest = keys.rest();
    let next = match rest {
        _ if rest.is_nil() => Term::static_atom("none"),
        _ if rest.is_nonempty_list() => Term::cons(rest.clone(), map.clone()),
        _ => return Ok(BifResult::badarg()),
    };
    let pairs = taken
        .iter()
        .map(|&key| entries.get(key).map(|value| (key, value)));
    let Some(pairs) = pairs.collect::<Option<Vec<_>>>() else {
        return Ok(BifResult::badarg());
    };
    if rest.is_nonempty_list() {
        process.bump_all_reductions();
    } else {
        process.bump_reductions((ORDERED_ITER_FACTOR * taken.len()) as i32);
    }
    Ok(Value(iterator_pairs(pairs.into_iter(), next)))
}

/// Build the iterator result `{K1, V1, {K2, V2, Next}}` of pairs, or `Next`
/// when there is no pair.
///
/// # Examples
///
/// ```rust,ignore
/// let (key, value) = (Term::Integer(1), test_atom("a"));
/// let result = iterator_pairs([(&key, &value)].into_iter(), test_atom("none"));
/// assert_eq!(result, Term::tuple(vec![key.clone(), value.clone(), test_atom("none")]));
/// ```
fn iterator_pairs<'a>(
    pairs: impl DoubleEndedIterator<Item = (&'a Term, &'a Term)>,
    next: Term,
) -> Term {
    pairs.rev().fold(next, |rest, (key, value)| {
        Term::tuple(vec![key.clone(), value.clone(), rest])
    })
}

/// Run `maps:merge/2`: return the first map with each pair of the second
/// map put into it, so the values of the second map win, as ERTS
/// `maps_merge_2` does (`erl_map.c:1257-1284`). Two copies of one map give
/// the first argument with no cost (`erl_map.c:1258-1262`). When the first
/// argument is not a map, raise `error:{badmap, First}`; else when the
/// second argument is not a map, raise `error:{badmap, Second}`.
///
/// Two flatmaps merge with no cost. Each other pair runs the steps of the
/// ERTS hashmap merge ([`Map::merge_steps`]) through [`merge_trap`].
///
/// # Examples
///
/// ```rust,ignore
/// let first = Term::Map(Map::from_pairs(vec![(test_atom("input"), test_atom("raw"))], &test_atom_table())?);
/// let second = Term::Map(Map::from_pairs(vec![(test_atom("input"), test_atom("cooked"))], &test_atom_table())?);
/// assert!(matches!(merge(&mut process, &first, &second, &test_atom_table())?, BifResult::Value(merged) if merged == second));
/// assert!(matches!(merge(&mut process, &test_atom("foo"), &second, &test_atom_table())?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when the merged map gets a key that the map form
/// does not support yet, for example an integer outside the small-integer
/// range in a map of more than 32 keys.
pub(super) fn merge(
    process: &mut ProcessState,
    first: &Term,
    second: &Term,
    atoms: &AtomTable,
) -> Result<BifResult, BeamError> {
    match (first, second) {
        (Term::Map(first_map), Term::Map(second_map)) if first_map.same_form(second_map) => {
            Ok(Value(first.clone()))
        }
        (Term::Map(first), Term::Map(second)) => {
            let pairs = first.iter().chain(second.iter());
            let pairs = pairs.map(|(key, value)| (key.clone(), value.clone()));
            let merged = Map::from_pairs(pairs.collect(), atoms)
                .map_err(|key| error(format!("unsupported map key {}", debug_term(&key, atoms))))?;
            let merged = Term::Map(merged);
            match first.merge_steps(second) {
                Some(steps) => merge_trap(process, merged, steps),
                None => Ok(Value(merged)),
            }
        }
        (Term::Map(_), _) => Ok(error_with("badmap", second)),
        _ => Ok(error_with("badmap", first)),
    }
}

/// Use the reductions of the hashmap merge that gave `merged`, of which
/// `steps` remain, as ERTS `hashmap_merge` does (`erl_map.c:1558-1851`), and
/// return `merged`. The call can take `MAP_MERGE_LOOP_FACTOR` (8) steps for
/// each reduction of the budget (`erl_map.c:1507`, `1569`). When the merge
/// ends before the budget ends, it bumps one reduction for each 8 steps,
/// rounded down (`erl_map.c:1823`). Else the step that uses the last of the
/// budget, or the first step for a budget of 0, traps: the BIF uses all
/// reductions and continues in the export `maps:merge_trap` with the rest
/// of the steps (`erl_map.c:1736-1739`, `1826-1850`). ERTS keeps the walk
/// state in a magic reference and traps to `maps:merge_trap/1`
/// (`erl_map.c:134-136`, `1498-1505`). PBeam merges the maps in the first
/// call, so the trap keeps the result and the steps that remain.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// assert!(matches!(merge_trap(&mut process, map.clone(), 79)?, BifResult::Value(_)));
/// assert_eq!(process.fcalls, 1);
/// process.fcalls = 10;
/// let trap = merge_trap(&mut process, map, 81)?;
/// assert!(matches!(trap, BifResult::TrapBif { function: "merge_trap", arguments, .. } if arguments[1] == Term::Integer(1)));
/// assert_eq!(process.fcalls, 0);
/// ```
pub(super) fn merge_trap(
    process: &mut ProcessState,
    merged: Term,
    steps: usize,
) -> Result<BifResult, BeamError> {
    /// `MAP_MERGE_LOOP_FACTOR` (`erl_map.c:1507`).
    const LOOP_FACTOR: usize = 8;
    // ERTS keeps the budget in a `Sint` and traps when `--reds <= 0`
    // (`erl_map.c:1569`, `1736`), so a negative budget traps at the first
    // step, as a budget of 0 does.
    let budget = (LOOP_FACTOR * process.bif_reductions_or_zero()).max(1);
    if steps >= budget {
        process.bump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "maps",
            function: "merge_trap",
            arguments: vec![merged, Term::Integer((steps - budget) as i64)],
        });
    }
    process.bump_reductions((steps / LOOP_FACTOR) as i32);
    Ok(Value(merged))
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
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?);
/// assert!(matches!(remove(&test_atom("a"), &map, &test_atom_table()), BifResult::Value(Term::Map(empty)) if empty.len() == 0));
/// assert!(matches!(remove(&test_atom("b"), &map, &test_atom_table()), BifResult::Value(same) if same == map));
/// assert!(matches!(remove(&test_atom("a"), &test_atom("foo"), &test_atom_table()), BifResult::Raise { .. }));
/// ```
pub(super) fn remove(key: &Term, map: &Term, atoms: &AtomTable) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    if entries.get(key).is_none() {
        return Value(map.clone());
    }
    Value(without(entries, key, atoms))
}

/// Return the map `entries` without the key `key`, which it holds, as
/// `erts_maps_take` builds it (`erl_map.c:1943-2034`).
///
/// # Examples
///
/// ```rust,ignore
/// let map = Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?;
/// assert!(matches!(without(&map, &test_atom("a"), &test_atom_table()), Term::Map(empty) if empty.len() == 0));
/// ```
fn without(entries: &Map, key: &Term, atoms: &AtomTable) -> Term {
    let pairs = entries.iter().filter(|(existing, _)| *existing != key);
    let pairs = pairs.map(|(key, value)| (key.clone(), value.clone()));
    // Each key of the rest is a key of the map, so the map form supports it.
    let rest =
        Map::from_pairs(pairs.collect(), atoms).expect("the keys of a map are valid map keys");
    Term::Map(rest)
}

/// Run `maps:take/2`: return `{Value, Rest}`, the value of the key and the
/// map without the key, or `error` for a key that the map does not hold, as
/// ERTS `maps_take_2` does with `erts_maps_take` (`erl_map.c:1910-1925`,
/// `1943-2034`). `Rest` is the map that [`remove`] gives. A term that is
/// not a map raises `error:{badmap, Map}`.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?);
/// assert!(matches!(take(&test_atom("a"), &map, &test_atom_table()), BifResult::Value(Term::Tuple(_))));
/// assert!(matches!(take(&test_atom("b"), &map, &test_atom_table()), BifResult::Value(Term::Atom(_))));
/// assert!(matches!(take(&test_atom("a"), &test_atom("foo"), &test_atom_table()), BifResult::Raise { .. }));
/// ```
pub(super) fn take(key: &Term, map: &Term, atoms: &AtomTable) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    let Some(value) = entries.get(key) else {
        return Value(Term::static_atom("error"));
    };
    Value(Term::tuple(vec![
        value.clone(),
        without(entries, key, atoms),
    ]))
}

/// Run `maps:update/3`: return the map with an existing key set to the
/// value, as ERTS `maps_update_3` does with `erts_maps_update`
/// (`erl_map.c:2036-2103`, `2241-2253`). The map keeps its form and the
/// order of its keys. A key that the map does not hold raises
/// `error:{badkey, Key}`, and a term that is not a map raises
/// `error:{badmap, Map}`. A key that the PBeam map form does not support is
/// not in the map, so it raises `error:{badkey, Key}` too (row "Map keys"
/// in `docs/erts-differences.md`).
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(1))], &test_atom_table())?);
/// let updated = Term::Map(Map::from_pairs(vec![(test_atom("a"), Term::Integer(2))], &test_atom_table())?);
/// assert!(matches!(update(&test_atom("a"), &Term::Integer(2), &map, &test_atom_table()), BifResult::Value(value) if value == updated));
/// assert!(matches!(update(&test_atom("b"), &Term::Integer(2), &map, &test_atom_table()), BifResult::Raise { .. }));
/// assert!(matches!(update(&test_atom("a"), &Term::Integer(2), &test_atom("foo"), &test_atom_table()), BifResult::Raise { .. }));
/// ```
pub(super) fn update(key: &Term, value: &Term, map: &Term, atoms: &AtomTable) -> BifResult {
    let Term::Map(entries) = map else {
        return error_with("badmap", map);
    };
    let mut entries = entries.clone();
    match entries.update(vec![(Insert::Set, key.clone(), value.clone())], atoms) {
        Ok(()) => Value(Term::Map(entries)),
        Err(key) => error_with("badkey", &key),
    }
}

/// Run `maps:values/1`: return the values of a map in map order, as ERTS
/// `maps_values_1` does (`erl_map.c:2262-2310`). A term that is not a map
/// raises `error:{badmap, Map}`.
///
/// ERTS runs the BIF as YCF code with 15 units for each reduction
/// (`erl_map.c:2299`; [`run_units`]), as `maps:keys/1` does. The loop of
/// `maps_values_1_helper` takes one unit for each key of a hashmap and no
/// unit for a flatmap (`erl_map.c:2262-2293`), and the loop of
/// `hashmap_values` takes one unit for each key (`erl_map.c:2844-2879`). A
/// yield traps to the export `maps:values_trap/2` with the map and the
/// units done, which continues with no cost in the next turn. ERTS traps to
/// `maps:values/1` with a magic reference to the YCF state.
///
/// # Examples
///
/// ```rust,ignore
/// let map = Term::Map(Map::from_pairs(vec![(Term::Integer(1), test_atom("a")), (Term::Integer(2), test_atom("b"))], &test_atom_table())?);
/// let values = Term::list_from([test_atom("a"), test_atom("b")]);
/// assert!(matches!(values(&mut process, &map, 0)?, BifResult::Value(result) if result == values));
/// assert!(matches!(values(&mut process, &Term::Integer(1), 0)?, BifResult::Raise { .. }));
/// ```
pub(super) fn values(
    process: &mut ProcessState,
    map: &Term,
    done: usize,
) -> Result<BifResult, BeamError> {
    /// `iterations_per_red` of `maps_values_1` (`erl_map.c:2299`).
    const PER_REDUCTION: usize = 15;
    let Term::Map(entries) = map else {
        return Ok(error_with("badmap", map));
    };
    let units = vec![
        1;
        if entries.is_hashmap() {
            entries.len()
        } else {
            0
        }
    ];
    match run_units(process, PER_REDUCTION, &units, done)? {
        YcfTurn::Done(reductions) => {
            process.bump_reductions(reductions);
            Ok(Value(Term::list_from(
                entries.iter().map(|(_, value)| value.clone()),
            )))
        }
        YcfTurn::Yield(done) => Ok(BifResult::TrapBif {
            module: "maps",
            function: "values_trap",
            arguments: vec![map.clone(), Term::Integer(done as i64)],
        }),
    }
}

/// Return the `error:{Tag, Term}` exception, as ERTS `BIF_ERROR` with
/// `BADMAP` or `BADKEY` and `fvalue` set to the term.
///
/// # Examples
///
/// ```rust,ignore
/// let BifResult::Raise { reason, .. } = error_with("badmap", &test_atom("foo")) else { panic!() };
/// assert_eq!(reason, Term::tuple(vec![test_atom("badmap"), test_atom("foo")]));
/// ```
pub(super) fn error_with(tag: &str, term: &Term) -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason: Term::tuple(vec![Term::static_atom(tag), term.clone()]),
    }
}
