//! Bodies of the `ets` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::ets::{Access, Operation, Table};

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
/// acts as `none`, as `set_heir` (`erl_db.c:5319-5337`) does. The table keeps
/// the last access mode, `protected` changes nothing (`erl_db.c:2705-2706`),
/// and the key position keeps only the low 32 bits, as the C `int`
/// `tb->common.keypos` (`erl_db_util.h:346`).
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
    let mut access = Access::Protected;
    let mut keypos = 1;
    let mut explicit_locks = false;
    let mut heir = None;
    for option in options {
        match option {
            Term::Atom(atom) => match atom.name() {
                "bag" | "duplicate_bag" | "ordered_set" => table_type = atom.name(),
                "named_table" => named = true,
                "public" => access = Access::Public,
                "private" => access = Access::Private,
                "set" | "protected" | "compressed" => {}
                _ => return Ok(BifResult::badarg()),
            },
            Term::Tuple(elements) => match elements.as_slice() {
                [Term::Atom(key), value] => match (key.name(), value) {
                    ("keypos", value) => match value.small_integer() {
                        // The C `int` keeps the low 32 bits (`erl_db_util.h:346`).
                        Some(value) if value > 0 => keypos = value as i32,
                        _ => return Ok(BifResult::badarg()),
                    },
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
    context.scheduler.ets.create(
        table,
        name.map(|name| name.name()),
        Table::new(owner, access, keypos),
    );
    Ok(Value(name.map_or(Term::Reference(table), |name| {
        Term::Atom(name.clone())
    })))
}

/// Run `ets:insert(Table, Objects)` on a `set` table and return `true`, as
/// ERTS `ets_insert_2` (`erl_db.c:2341-2381`) does. `Objects` is one tuple
/// or a list of tuples. An object with a key that is exactly equal to the key
/// of an object in the table replaces that object, also an earlier object of
/// the same list (`erl_db_hash.c:1387-1402`).
///
/// A table that does not exist or that the caller cannot write raises
/// `badarg` (`erl_db.c:769-798`), also for `[]`. A term that is not a tuple,
/// and a tuple with fewer elements than the key position, raise `badarg`,
/// and a list with such an element adds no object
/// (`ets_insert_2_list_check`, `erl_db.c:1812-1827`). ERTS compares the
/// unsigned size with the C `int` key position, so a negative key position
/// makes each object bad.
///
/// # Examples
///
/// ```rust,ignore
/// let table = new(&mut context, &test_atom("t"), &Term::List(vec![]))?;
/// let BifResult::Value(table) = table else { unreachable!() };
/// let object = Term::Tuple(vec![test_atom("k"), test_atom("v")]);
/// assert!(matches!(insert(&mut context, &table, &object)?, BifResult::Value(_)));
/// assert!(matches!(insert(&mut context, &table, &test_atom("x"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error for a tuple in a table with the key position 0:
/// ERTS aborts the emulator there (`Invalid tag in make_internal_hash`).
pub(super) fn insert(
    context: &mut BifContext<'_>,
    table: &Term,
    objects: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) = context.scheduler.ets.table(table, caller, Operation::Write) else {
        return Ok(BifResult::badarg());
    };
    let objects = match objects {
        Term::List(objects) => objects.as_slice(),
        object => std::slice::from_ref(object),
    };
    // A C `int` compares with the unsigned arity as a 64-bit unsigned value.
    let keypos = i64::from(table.keypos()) as u64;
    let mut tuples = Vec::with_capacity(objects.len());
    for object in objects {
        match object {
            Term::Tuple(elements) if elements.len() as u64 >= keypos => tuples.push(elements),
            _ => return Ok(BifResult::badarg()),
        }
    }
    if keypos == 0 && !tuples.is_empty() {
        // ponytail: ERTS aborts here; see "ETS tables" in docs/erts-differences.md.
        return Err(error(
            "unsupported ets:insert/2 in a table with key position 0",
        ));
    }
    for elements in tuples {
        table.insert(elements.clone());
    }
    Ok(Value(Term::static_atom("true")))
}

/// Run `ets:lookup_element(Table, Key, Index, Default)` on a `set` table, as
/// ERTS `ets_lookup_element_4` (`erl_db.c:2972-3001`) does: return the
/// element at `Index` of the object with the key `Key`, or `Default` when
/// the table has no object with that key.
///
/// A table that does not exist or that the caller cannot read raises
/// `badarg` first. Then an index that is not a small integer or that is
/// below 1 raises `badarg`. ERTS gives the index to `db_get_element` as a C
/// `int` (`erl_db_util.h:151-155`), so only its low 32 bits count. For a
/// found object, a negative or too large position raises `badarg`
/// (`erl_db_hash.c:1563-1566`).
///
/// # Examples
///
/// ```rust,ignore
/// let key = test_atom("k");
/// let result = lookup_element(&mut context, &table, &key, &Term::Integer(2), &test_atom("d"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("v")));
/// let result = lookup_element(&mut context, &table, &key, &Term::Integer(0), &test_atom("d"))?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error for a found object and an index whose low 32 bits
/// are 0: ERTS reads the tuple header there and crashes.
pub(super) fn lookup_element(
    context: &mut BifContext<'_>,
    table: &Term,
    key: &Term,
    index: &Term,
    default: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) = context.scheduler.ets.table(table, caller, Operation::Read) else {
        return Ok(BifResult::badarg());
    };
    let Some(index) = index.small_integer().filter(|index| *index >= 1) else {
        return Ok(BifResult::badarg());
    };
    let Some(object) = table.object(key) else {
        return Ok(Value(default.clone()));
    };
    let position = index as i32;
    if position == 0 {
        // ponytail: ERTS crashes here; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!(
            "unsupported ets:lookup_element/4 index {index} with position 0"
        )));
    }
    Ok(usize::try_from(position)
        .ok()
        .and_then(|position| object.get(position - 1))
        .map_or_else(BifResult::badarg, |element| Value(element.clone())))
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
