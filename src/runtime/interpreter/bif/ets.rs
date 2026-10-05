//! Bodies of the `ets` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;

/// The fewest and the most locks that the hidden option
/// `{write_concurrency, {debug_hash_fixed_number_of_locks, N}}` accepts
/// (`DB_WRITE_CONCURRENCY_MIN_LOCKS` and `DB_WRITE_CONCURRENCY_MAX_LOCKS`,
/// `erl_db.c:65-66`).
const LOCKS: std::ops::RangeInclusive<i64> = 1..=32768;

/// Run `ets:new(Name, Options)`: make a `set` table that the calling process
/// owns, and return `Name` for a named table or a new table reference for
/// another table, as ERTS `ets_new_2` (`erl_db.c:2555-2840`) does.
///
/// The checks keep the ERTS order, so PBeam raises `badarg` for each call
/// that raises it on ERTS:
///
/// 1. A name that is not an atom or options that are not a list raise
///    `badarg` (`erl_db.c:2575-2580`).
/// 2. Each option that the option loop (`erl_db.c:2597-2710`) does not accept
///    raises `badarg` (`erl_db.c:2711-2713`). It accepts `set`, `bag`,
///    `duplicate_bag`, `ordered_set`, `public`, `protected`, `private`,
///    `named_table`, `compressed`, `{keypos, N}` with a small integer
///    `N > 0`, `{write_concurrency, true | false | auto}`,
///    `{write_concurrency, {debug_hash_fixed_number_of_locks, 1..32768}}`,
///    `{read_concurrency, Bool}`, `{decentralized_counters, Bool}`,
///    `{heir, none}`, `{heir, Pid}`, and `{heir, Pid, Data}`. A later type
///    option replaces an earlier one, and `set` changes nothing, so
///    `[bag, set]` makes a `bag` (`erl_db.c:2599-2611`, `erl_db.c:2705-2706`).
/// 3. The final type `ordered_set` with `debug_hash_fixed_number_of_locks`
///    as the last `write_concurrency` option raises `badarg`
///    (`erl_db.c:2717-2758`).
/// 4. A name that a live named table has raises `badarg`
///    (`erl_db.c:2821-2831`).
///
/// Then the final type `bag`, `duplicate_bag`, or `ordered_set`, and a heir
/// that is a live process, also the caller, give a runtime error: ERTS makes
/// such a table, and PBeam cannot model it yet. A heir that is not alive
/// acts as `none`, as `set_heir` (`erl_db.c:5319-5337`) does. PBeam does not
/// store the access mode and the key position yet, because no BIF reads them.
///
/// # Examples
///
/// ```rust,ignore
/// let result = new(&mut context, &test_atom("ac_tab"), &Term::List(vec![test_atom("named_table")]))?;
/// assert!(matches!(result, BifResult::Value(name) if name == test_atom("ac_tab")));
/// let result = new(&mut context, &test_atom("ac_tab"), &Term::List(vec![]))?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// let result = new(&mut context, &test_atom("ac_tab"), &Term::List(vec![test_atom("bogus")]))?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// assert!(new(&mut context, &test_atom("ac_tab"), &Term::List(vec![test_atom("bag")])).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for a table type other than `set`, and for a heir
/// that is a live process.
pub(super) fn new(
    context: &mut BifContext<'_>,
    name: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(name), Term::List(options)) = (name, options) else {
        return Ok(BifResult::badarg());
    };
    let mut table_type = "set";
    let mut named = false;
    let mut explicit_locks = false;
    let mut heir = None;
    for option in options {
        match option {
            Term::Atom(atom) => match atom.name() {
                "bag" | "duplicate_bag" | "ordered_set" => table_type = atom.name(),
                "named_table" => named = true,
                "set" | "protected" | "public" | "private" | "compressed" => {}
                _ => return Ok(BifResult::badarg()),
            },
            Term::Tuple(elements) => match elements.as_slice() {
                [Term::Atom(key), value] => match (key.name(), value) {
                    ("keypos", keypos) if keypos.small_integer().is_some_and(|n| n > 0) => {}
                    ("write_concurrency", Term::Atom(value))
                        if matches!(value.name(), "true" | "false" | "auto") =>
                    {
                        explicit_locks = false;
                    }
                    ("write_concurrency", Term::Tuple(locks)) if is_lock_count(locks) => {
                        explicit_locks = true;
                    }
                    ("read_concurrency" | "decentralized_counters", Term::Atom(value))
                        if matches!(value.name(), "true" | "false") => {}
                    ("heir", Term::Atom(value)) if value.name() == "none" => heir = None,
                    ("heir", Term::Pid(pid)) => heir = Some(*pid),
                    _ => return Ok(BifResult::badarg()),
                },
                [Term::Atom(key), Term::Pid(pid), _] if key.name() == "heir" => heir = Some(*pid),
                _ => return Ok(BifResult::badarg()),
            },
            _ => return Ok(BifResult::badarg()),
        }
    }
    if (table_type == "ordered_set" && explicit_locks)
        || (named && context.scheduler.ets.name_in_use(name.name()))
    {
        return Ok(BifResult::badarg());
    }
    if table_type != "set" {
        // ponytail: only set tables; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!(
            "unsupported ets:new/2 table type {table_type}"
        )));
    }
    if let Some(heir) = heir
        && context.scheduler.process(heir).is_some()
    {
        // ponytail: no table transfer to a heir; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!("unsupported ets:new/2 heir {heir:?}")));
    }
    let table = context.scheduler.make_ref()?;
    let owner = context.scheduler.current_process_mut().id;
    let name = named.then_some(name);
    context
        .scheduler
        .ets
        .create(table, owner, name.map(|name| name.name()));
    Ok(Value(name.map_or(Term::Reference(table), |name| {
        Term::Atom(name.clone())
    })))
}

/// Return whether `{Tag, Count}` is the hidden lock option
/// `{debug_hash_fixed_number_of_locks, Count}` with a count in `LOCKS`, as
/// `ets_new_2` checks it (`erl_db.c:2639-2655`).
///
/// # Examples
///
/// ```rust,ignore
/// let tag = test_atom("debug_hash_fixed_number_of_locks");
/// assert!(is_lock_count(&[tag.clone(), Term::Integer(1)]));
/// assert!(!is_lock_count(&[tag, Term::Integer(0)]));
/// ```
fn is_lock_count(locks: &[Term]) -> bool {
    matches!(locks, [Term::Atom(tag), Term::Integer(count)]
        if tag.name() == "debug_hash_fixed_number_of_locks" && LOCKS.contains(count))
}
