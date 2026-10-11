//! Bodies of the `ets` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::ets::{Access, Operation, Table};
use crate::term::Reference;

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
/// that is a live process other than the caller, give a runtime error: ERTS
/// makes such a table, and PBeam cannot model it yet. A heir that is not
/// alive acts as `none`, as `set_heir` (`erl_db.c:5319-5337`) does. A heir
/// that is the caller also acts as `none`: at the owner exit, ERTS gives the
/// table to no process when the heir is the owner (`erl_db.c:5076-5078`),
/// and no BIF that PBeam has reads the heir. The table keeps
/// the last access mode, `protected` changes nothing (`erl_db.c:2705-2706`),
/// and the key position keeps only the low 32 bits, as the C `int`
/// `tb->common.keypos` (`erl_db_util.h:346`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = new(&mut context, &test_atom("ac_tab"), &Term::list_from(vec![test_atom("named_table")]))?;
/// assert!(matches!(result, BifResult::Value(name) if name == test_atom("ac_tab")));
/// let result = new(&mut context, &test_atom("ac_tab"), &Term::nil())?;
/// assert!(matches!(result, BifResult::Value(Term::Reference(_))));
/// let result = new(&mut context, &test_atom("ac_tab"), &Term::list_from(vec![test_atom("bogus")]))?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// assert!(new(&mut context, &test_atom("ac_tab"), &Term::list_from(vec![test_atom("bag")])).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for a table type other than `set`, and for a heir
/// that is a live process other than the caller.
pub(super) fn new(
    context: &mut BifContext<'_>,
    name: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(name), Some(options)) = (name, options.proper_list()) else {
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
    let owner = context.scheduler.current_process_mut().id;
    if let Some(heir) = heir
        && heir != owner
        && context.scheduler.process(heir).is_some()
    {
        // ponytail: no table transfer to a heir; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!("unsupported ets:new/2 heir {heir:?}")));
    }
    let table = context.scheduler.make_ref()?;
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
/// One tuple, `[]`, and a list of one tuple cost no reduction in a `set`
/// table (`erl_db.c:2349-2380`). A table that does not exist or that the
/// caller cannot write raises `badarg` (`erl_db.c:769-798`). A term that is
/// not a tuple, and a tuple with fewer elements than the key position, raise
/// `badarg`. ERTS compares the unsigned size with the C `int` key position,
/// so a negative key position makes each object bad. Each other list,
/// also an improper list, runs [`insert_list`].
///
/// # Examples
///
/// ```rust,ignore
/// let table = new(&mut context, &test_atom("t"), &Term::nil())?;
/// let BifResult::Value(table) = table else { unreachable!() };
/// let object = Term::Tuple(vec![test_atom("k"), test_atom("v")]);
/// assert!(matches!(insert(&mut context, &table, &object)?, BifResult::Value(_)));
/// assert!(matches!(insert(&mut context, &table, &test_atom("x"))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error for a tuple in a table with the key position 0:
/// ERTS aborts the emulator there (`Invalid tag in make_internal_hash`). Also return the runtime error of
/// [`ProcessState::bif_reductions_left`] for a negative budget.
pub(super) fn insert(
    context: &mut BifContext<'_>,
    table: &Term,
    objects: &Term,
) -> Result<BifResult, BeamError> {
    // A cons cell with a tail that is not `[]`: at least two objects, or an
    // improper list.
    if objects.cons_cell().is_some_and(|(_, tail)| !tail.is_nil()) {
        let Some((id, found)) = context.scheduler.ets.find(table) else {
            return Ok(BifResult::badarg());
        };
        let keypos = found.keypos();
        return insert_list(context, id, objects, 0, keypos);
    }
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) = context.scheduler.ets.table(table, caller, Operation::Write) else {
        return Ok(BifResult::badarg());
    };
    let objects: Vec<&Term> = match objects.proper_list() {
        Some(objects) => objects.collect(),
        None => vec![objects],
    };
    let keypos = table.keypos();
    let Some(tuples) = valid_objects(objects, keypos) else {
        return Ok(BifResult::badarg());
    };
    put_objects(table, &tuples, keypos)?;
    Ok(Value(Term::static_atom("true")))
}

/// Run `ets:insert/2` with a list of at least two objects, or an improper
/// list, from `done` units of its work, as the YCF code of
/// `ets_insert_2_list` does (`erl_db.c:2017-2116`, `2152-2336`). The table
/// `id` has the key position `keypos` at the first call. The work has
/// units, and a unit uses one of the `ITERATIONS_PER_RED` (8) units that
/// each reduction of the budget gives:
///
/// 1. The check: one unit for each object up to the first bad object
///    (`ets_insert_2_list_check`, `erl_db.c:1812-1827`). A bad object, or a
///    tail that is not `[]` after the last object, raises `badarg`.
/// 2. When the units that remain are at least the number of objects
///    (`can_insert_without_yield`, `erl_db.c:2002-2014`), the caller must
///    be able to write the table, and the objects go into it with two units
///    for each object and no trap (`erl_db.c:1845-1859`, `2050-2071`). The
///    cost can then be more than the budget, and `BUMP_REDS` stops at 0.
/// 3. Else a copy of each object, one unit for each object
///    (`erl_db.c:1876-1907`). Then the caller must be able to write the
///    table, and the objects go into it with two units for each object
///    (`erl_db.c:1927-1949`, `2072-2107`). When the caller cannot write the
///    table, or the table went away, the copies go away with one unit for
///    each object, and the call raises `badarg` (`erl_db.c:1863-1873`,
///    `2077-2089`).
///
/// The call bumps one reduction for each 8 units that it used
/// (`erl_db.c:2197`, `2306`). When the unit that uses the last of the
/// budget, or the first unit for a budget of 0, is not the last unit, the
/// call uses all reductions and traps to its export with `done` after that
/// unit (`erl_db.c:2297-2332`). ERTS traps to `ets:insert/2` with a magic
/// reference to the YCF state, and PBeam traps to the internal export
/// `ets:insert/4` with the table reference, the objects, `done`, and the key
/// position. `done` counts the units of the check, the copy, and the
/// insert, and after a failed copy the units of the removal from
/// `4 * count`. ERTS takes a table lock and inserts each object in step 3, and
/// another process that uses the table helps to complete the insert. PBeam
/// inserts all objects when it takes the table, so another process sees
/// them at once. When the owner exit deletes the table during step 3, a
/// later call raises `badarg`, as ERTS does after it cancels the insert
/// (`erl_db.c:2232-2248`, `5082-5086`). PBeam has no `ets:delete/1`, which
/// in ERTS first completes the insert, so that the insert returns `true`.
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let objects = Term::list_from(vec![Term::Tuple(vec![test_atom("k")]); 8]);
/// // 8 units of the check and 16 units of the insert: 3 reductions.
/// assert!(matches!(insert_list(&mut context, id, &objects, 0, 1)?, BifResult::Value(_)));
/// assert_eq!(context.scheduler.current_process_mut().fcalls, 7);
/// ```
///
/// # Errors
///
/// Return a runtime error for a table with the key position 0: ERTS aborts
/// the emulator there (`Invalid tag in make_internal_hash`). Also return the runtime error of
/// [`ProcessState::bif_reductions_left`] for a negative budget.
pub(super) fn insert_list(
    context: &mut BifContext<'_>,
    id: Reference,
    objects: &Term,
    mut done: usize,
    keypos: i32,
) -> Result<BifResult, BeamError> {
    /// `ITERATIONS_PER_RED` (`erl_db.c:2152`).
    const ITERATIONS_PER_RED: i64 = 8;
    let (elements, tail) = objects.list_cells().expect("a list insert has a list");
    let elements: Vec<&Term> = elements.collect();
    let count = elements.len();
    let table = Term::Reference(id);
    let insert_phase = 2 * count < done && done <= 4 * count;
    if insert_phase && context.scheduler.ets.find(&table).is_none() {
        return Ok(BifResult::badarg());
    }
    let process = context.scheduler.current_process_mut();
    let caller = process.id;
    let budget = ITERATIONS_PER_RED * process.bif_reductions_left()? as i64;
    let mut units = Units { left: budget };
    let tuples = valid_objects(elements.iter().copied(), keypos).filter(|_| tail.is_none());
    // After a failed copy, the units of the removal of the copies follow
    // `4 * count`, and a removal that yields leaves `done` above it.
    let mut removal = done > 4 * count;
    // `None` when the call yields, else the result of the call.
    let result = 'work: {
        if done <= count {
            let check_end = match &tuples {
                Some(_) => count,
                None => (elements.iter())
                    .position(|object| valid_objects([*object], keypos).is_none())
                    .map_or(count, |index| index + 1),
            };
            if !units.run(&mut done, check_end) {
                break 'work None;
            }
            let Some(tuples) = &tuples else {
                break 'work Some(BifResult::badarg());
            };
            if count as i64 <= units.left {
                let Some(table) = context
                    .scheduler
                    .ets
                    .table(&table, caller, Operation::Write)
                else {
                    break 'work Some(BifResult::badarg());
                };
                put_objects(table, tuples, keypos)?;
                units.left -= 2 * count as i64;
                break 'work Some(Value(Term::static_atom("true")));
            }
        }
        if done <= 2 * count {
            if !units.run(&mut done, 2 * count) {
                break 'work None;
            }
            match context
                .scheduler
                .ets
                .table(&table, caller, Operation::Write)
            {
                Some(table) => {
                    let tuples = tuples.expect("the check of a copied list found no bad object");
                    put_objects(table, &tuples, keypos)?;
                }
                None => (removal, done) = (true, 4 * count),
            }
        }
        if !removal {
            if !units.run(&mut done, 4 * count) {
                break 'work None;
            }
            break 'work Some(Value(Term::static_atom("true")));
        }
        if !units.run(&mut done, 5 * count) {
            break 'work None;
        }
        Some(BifResult::badarg())
    };
    let process = context.scheduler.current_process_mut();
    process.bump_reductions(((budget - units.left) / ITERATIONS_PER_RED) as i32);
    Ok(result.unwrap_or_else(|| BifResult::TrapBif {
        module: "ets",
        function: "insert",
        arguments: vec![
            table,
            objects.clone(),
            Term::Integer(done as i64),
            Term::Integer(i64::from(keypos)),
        ],
    }))
}

/// The YCF reduction counter of a call (`YCF_NR_OF_REDS_LEFT`): the units
/// that remain in the turn.
struct Units {
    /// The units that remain. It can be 0 or less after the last unit.
    left: i64,
}

impl Units {
    /// Use the units from `done` to `end`, and return true. When a unit
    /// makes the counter 0 or less before `end`, as `YCF_CONSUME_REDS` and
    /// the YCF loop step do, stop after that unit, set `done` to it, and
    /// return false: the call yields there. A counter of 0 or less yields
    /// after the first unit.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut units, mut done) = (Units { left: 3 }, 0);
    /// assert!(units.run(&mut done, 2));
    /// assert!(!units.run(&mut done, 5));
    /// assert_eq!((done, units.left), (3, 0));
    /// ```
    fn run(&mut self, done: &mut usize, end: usize) -> bool {
        let units = (end - *done) as i64;
        let limit = self.left.max(1);
        if units >= limit {
            *done += limit as usize;
            self.left -= limit;
            return false;
        }
        *done = end;
        self.left -= units;
        true
    }
}

/// Put each object of `tuples` into `table`, which has the key position
/// `keypos`.
///
/// # Examples
///
/// ```rust,ignore
/// let object = vec![test_atom("k")];
/// put_objects(&mut table, &[&object], 1)?;
/// assert!(table.object(&test_atom("k")).is_some());
/// ```
///
/// # Errors
///
/// Return a runtime error for an object in a table with the key position
/// 0: ERTS aborts the emulator there (`Invalid tag in make_internal_hash`).
fn put_objects(table: &mut Table, tuples: &[&Vec<Term>], keypos: i32) -> Result<(), BeamError> {
    if keypos == 0 && !tuples.is_empty() {
        // ponytail: ERTS aborts here; see "ETS tables" in docs/erts-differences.md.
        return Err(error(
            "unsupported ets:insert/2 in a table with key position 0",
        ));
    }
    for elements in tuples {
        table.insert((*elements).clone());
    }
    Ok(())
}

/// Return the tuples of `objects` when each one has at least `keypos`
/// elements, as `ets_insert_2_list_check` (`erl_db.c:1812-1827`) checks
/// them. A C `int` key position compares with the unsigned arity as a
/// 64-bit unsigned value, so a negative key position makes each object bad.
///
/// # Examples
///
/// ```rust,ignore
/// let object = Term::Tuple(vec![test_atom("k")]);
/// assert!(valid_objects([&object], 1).is_some());
/// assert!(valid_objects([&object], 2).is_none());
/// assert!(valid_objects([&object], -1).is_none());
/// ```
fn valid_objects<'a>(
    objects: impl IntoIterator<Item = &'a Term>,
    keypos: i32,
) -> Option<Vec<&'a Vec<Term>>> {
    let keypos = i64::from(keypos) as u64;
    objects
        .into_iter()
        .map(|object| match object {
            Term::Tuple(elements) if elements.len() as u64 >= keypos => Some(elements),
            _ => None,
        })
        .collect()
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

/// Run `ets:lookup(Table, Key)` on a `set` table, as ERTS `ets_lookup_2`
/// (`erl_db.c:2877-2900`) and `db_get_hash` (`erl_db_hash.c:1492-1516`) do:
/// return `[Object]` for the object with a key that is exactly equal to
/// `Key`, or `[]`. A table that does not exist or that the caller cannot
/// read raises `badarg` (`erl_db.c:769-912`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = lookup(&mut context, &table, &test_atom("k"));
/// assert!(matches!(result, BifResult::Value(objects) if objects.proper_list().is_some_and(|objects| objects.count() == 1)));
/// assert!(matches!(lookup(&mut context, &test_atom("missing"), &test_atom("k")), BifResult::Raise { .. }));
/// ```
pub(super) fn lookup(context: &mut BifContext<'_>, table: &Term, key: &Term) -> BifResult {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) = context.scheduler.ets.table(table, caller, Operation::Read) else {
        return BifResult::badarg();
    };
    let object = table.object(key).map(|object| Term::Tuple(object.to_vec()));
    Value(Term::list_from(object))
}

/// Run `ets:member(Table, Key)` on a `set` table, as ERTS `ets_member_2`
/// (`erl_db.c:2905-2928`) and `db_member_hash` (`erl_db_hash.c:1518-1542`)
/// do: return `true` when the table has an object with a key that is
/// exactly equal to `Key`, else `false`. A table that does not exist or that
/// the caller cannot read raises `badarg` (`erl_db.c:769-912`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = member(&mut context, &table, &test_atom("k"));
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("true")));
/// assert!(matches!(member(&mut context, &Term::Integer(1), &test_atom("k")), BifResult::Raise { .. }));
/// ```
pub(super) fn member(context: &mut BifContext<'_>, table: &Term, key: &Term) -> BifResult {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) = context.scheduler.ets.table(table, caller, Operation::Read) else {
        return BifResult::badarg();
    };
    let found = table.object(key).is_some();
    Value(Term::static_atom(if found { "true" } else { "false" }))
}

/// Run `ets:delete(Table, Key)` on a `set` table and return `true`, as ERTS
/// `ets_delete_2` (`erl_db.c:3328-3349`) and `db_erase_hash`
/// (`erl_db_hash.c:1613-1664`) do. The object with a key that is exactly
/// equal to `Key` goes out of the table. A key that the table does not hold
/// also returns `true`. A table that does not exist or that the caller
/// cannot write raises `badarg` (`erl_db.c:769-912`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = delete(&mut context, &table, &test_atom("k"));
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("true")));
/// assert!(matches!(delete(&mut context, &Term::nil(), &test_atom("k")), BifResult::Raise { .. }));
/// ```
pub(super) fn delete(context: &mut BifContext<'_>, table: &Term, key: &Term) -> BifResult {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) = context.scheduler.ets.table(table, caller, Operation::Write) else {
        return BifResult::badarg();
    };
    table.delete(key);
    Value(Term::static_atom("true"))
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
