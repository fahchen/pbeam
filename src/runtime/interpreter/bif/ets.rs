//! Bodies of the `ets` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::ets::pattern::{MatchSpec, SpecError};
use crate::runtime::ets::{Access, Busy, EtsTables, ITERATIONS_PER_RED, Lacking, Table, TableType};
use crate::term::Reference;
use num_bigint::BigInt;
use std::sync::Arc;

mod tree;

/// The fewest and the most locks that the hidden option
/// `{write_concurrency, {debug_hash_fixed_number_of_locks, N}}` accepts
/// (`DB_WRITE_CONCURRENCY_MIN_LOCKS` and `DB_WRITE_CONCURRENCY_MAX_LOCKS`,
/// `erl_db.c:65-66`).
const LOCKS: std::ops::RangeInclusive<i64> = 1..=32768;

/// Run up to `units` units of the steps of the list insert that took the
/// table `table`, as `ets_insert_2_list_continue` runs the steps of a
/// trapped call (`erl_db.c:2173-2197`): the walk of `ets:insert_new/2` and
/// the insert of the objects, one step for each object. The steps are in
/// the table, so a call that traps between two objects leaves the position
/// there, and another process that uses the table runs the steps too
/// ([`crate::runtime::ets::EtsTables::read`]). Return the units that the steps used, and the result
/// when the insert ended, else `None`.
fn step_inserting(
    context: &mut BifContext<'_>,
    table: &Term,
    units: i64,
    mine: Option<ProcessId>,
) -> Result<(i64, Option<Term>), BeamError> {
    let scheduler = &mut *context.scheduler;
    let ets = scheduler.ets.shared();
    // The table of the caller has the list insert that it did not finish:
    // the step of a call of the table, or the completion of the insert,
    // clears it (`erl_db.c:2185-2195`). A process of another scheduler can
    // complete the insert or delete the table first, and then the step does
    // nothing.
    let Some(mut inserting) = (ets.id(&scheduler.atoms, table))
        .and_then(|id| ets.busy(&scheduler.atoms, &Term::Reference(id)))
    else {
        return Ok((0, None));
    };
    let owner = inserting
        .busy
        .as_ref()
        .expect("the caller found the busy insert")
        .owner;
    // The process that started the insert is the only one that gets its
    // result: a later insert of the same table takes the table after the
    // call of the earlier one returned (`erl_db.c:2223-2253`).
    if mine.is_some_and(|caller| owner != caller) {
        return Ok((0, None));
    }
    let (used, result) = inserting.step_busy(&scheduler.atoms, units);
    if let Some(value) = result {
        // The state of the insert lives with the process that started it,
        // so the result stays for it after the table goes away, and the
        // process reads it in its next turn (`erl_db.c:2142-2150`,
        // `2232-2244`). The process can run on another scheduler, so the
        // result waits in the table store. It goes there before the table
        // lock goes, as ERTS sets `return_value` before it clears
        // `DB_BUSY` (`erl_db.c:2183-2192`).
        if scheduler.process(owner).is_some() || scheduler.remote_owner(owner).is_some() {
            ets.put_result(owner, value);
        }
    }
    Ok((used, None))
}

/// Return the result of a BIF that could not take its table: `badarg`, or
/// a trap that runs the call `ets:function(arguments)` again after the
/// help of a busy table, which charges the reductions of the help
/// (`handle_lacking_permission`, `erl_db.c:769-798`, `2152-2197`).
fn retry(
    context: &mut BifContext<'_>,
    lacking: Lacking,
    function: &'static str,
    arguments: Vec<Term>,
) -> BifResult {
    let reductions = match lacking {
        Lacking::Busy { reductions } => reductions,
        Lacking::Denied(cause) => return denied(cause),
    };
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
    BifResult::TrapBif {
        module: "ets",
        function,
        arguments,
    }
}

/// Return the `error:badarg` exception with the `error_info` cause `cause`,
/// as the ETS BIFs raise it with `BADARG | EXF_HAS_EXT_INFO` and
/// `p->fvalue = Cause` (`erl_db.c:53-63`; `beam_common.c:993-1002`).
fn denied(cause: &'static str) -> BifResult {
    BifResult::ErrorWithCause {
        reason: Term::static_atom("badarg"),
        cause: Term::static_atom(cause),
    }
}

/// Run `ets:new(Name, Options)`: make a hash table that the calling process
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
/// Then the final type `ordered_set`, and a heir that is a live process
/// other than the caller, give a runtime error: ERTS makes such a table, and
/// PBeam cannot model it yet. The final type `set`, `bag`, or
/// `duplicate_bag` gives a hash table of that type. A heir that is not
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
/// assert!(new(&mut context, &test_atom("ac_tab"), &Term::list_from(vec![test_atom("ordered_set")])).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for the table type `ordered_set`, and for a heir
/// that is a live process other than the caller.
pub(super) fn new(
    context: &mut BifContext<'_>,
    name: &Term,
    options: &Term,
) -> Result<BifResult, BeamError> {
    let (Term::Atom(name), Some(options)) = (name, options.proper_list()) else {
        return Ok(BifResult::badarg());
    };
    let mut table_type = TableType::Set;
    let mut named = false;
    let mut access = Access::Protected;
    let mut keypos = 1;
    let mut explicit_locks = false;
    let mut heir = None;
    let mut read_concurrency = false;
    let mut compressed = false;
    // The `write_concurrency` option of a hash table: the value that
    // `ets:info/2` gives for a fine-locked table, whether the table is fine
    // locked, and whether its counters are decentralized
    // (`erl_db.c:2585-2658`, `2714-2716`). An `ordered_set` option sets
    // decentralized counters, and a later `{write_concurrency, true}` keeps
    // them (`erl_db.c:2607-2629`).
    let mut concurrency = Term::static_atom("false");
    let mut decentralized = false;
    let mut decentralized_option = None;
    for option in options {
        match option {
            Term::Atom(atom) => {
                if atom.is_static("bag") {
                    table_type = TableType::Bag;
                } else if atom.is_static("duplicate_bag") {
                    table_type = TableType::DuplicateBag;
                } else if atom.is_static("ordered_set") {
                    table_type = TableType::OrderedSet;
                    decentralized = true;
                } else if atom.is_static("named_table") {
                    named = true;
                } else if atom.is_static("public") {
                    access = Access::Public;
                } else if atom.is_static("private") {
                    access = Access::Private;
                } else if atom.is_static("compressed") {
                    compressed = true;
                } else if atom.is_static("set") || atom.is_static("protected") {
                } else {
                    return Ok(BifResult::badarg());
                }
            }
            Term::Tuple(elements) => match elements.as_ref() {
                [Term::Atom(key), value] => match (context.scheduler.atoms.name_of(key), value) {
                    ("keypos", value) => match value.small_integer() {
                        // The C `int` keeps the low 32 bits (`erl_db_util.h:346`).
                        Some(value) if value > 0 => keypos = value as i32,
                        _ => return Ok(BifResult::badarg()),
                    },
                    ("write_concurrency", Term::Atom(value))
                        if value.is_one_of(&["true", "false", "auto"]) =>
                    {
                        explicit_locks = false;
                        if value.is_static("auto") {
                            decentralized = true;
                        } else if value.is_static("true") && table_type != TableType::OrderedSet {
                            decentralized = false;
                        }
                        concurrency = Term::Atom(value.clone());
                    }
                    ("write_concurrency", Term::Tuple(locks)) if is_lock_count(locks) => {
                        explicit_locks = true;
                        decentralized = true;
                        // ERTS rounds the count down to a power of two, and
                        // at least 64 (`erl_db_hash.c:1043-1050`).
                        let count = locks[1].small_integer().unwrap_or(1);
                        let count = (1_i64 << count.ilog2()).max(64);
                        concurrency = Term::tuple(vec![locks[0].clone(), Term::Integer(count)]);
                    }
                    ("read_concurrency", Term::Atom(value))
                        if value.is_one_of(&["true", "false"]) =>
                    {
                        read_concurrency = value.is_static("true");
                    }
                    ("decentralized_counters", Term::Atom(value))
                        if value.is_one_of(&["true", "false"]) =>
                    {
                        decentralized_option = Some(value.is_static("true"));
                    }
                    ("heir", Term::Atom(value)) if value.is_static("none") => {
                        heir = None;
                    }
                    ("heir", Term::Pid(pid)) => heir = Some(*pid),
                    _ => return Ok(BifResult::badarg()),
                },
                [Term::Atom(key), Term::Pid(pid), _] if key.is_static("heir") => {
                    heir = Some(*pid);
                }
                _ => return Ok(BifResult::badarg()),
            },
            _ => return Ok(BifResult::badarg()),
        }
    }
    if table_type == TableType::OrderedSet && explicit_locks {
        return Ok(BifResult::badarg());
    }
    let owner = context.scheduler.current_process_mut().id;
    let table = context.scheduler.make_ref()?.table_reference();
    let mut made = Table::new(owner, access, keypos, table_type);
    made.set_name(context.scheduler.atoms.name_of(name));
    let name = named.then_some(name);
    // A run with one scheduler gives no fine locking (`DB_LOCK_FREE`,
    // `erl_db.c:2656-2657`), and neither does a `private` table
    // (`erl_db.c:2743`). A table that is not fine locked has centralized
    // counters (`erl_db.c:2776-2779`), so `ets:info/2` gives `false` for
    // `write_concurrency` and `decentralized_counters`. PBeam keeps one
    // lock for each table in each case.
    let fine = context.scheduler.is_multi()
        && !matches!(access, Access::Private)
        && concurrency != Term::static_atom("false");
    let decentralized = fine && decentralized_option.unwrap_or(decentralized);
    made.set_options(
        if fine {
            concurrency
        } else {
            Term::static_atom("false")
        },
        read_concurrency,
        decentralized,
        compressed,
    );
    // A heir that is not alive, or that is another live process, acts as
    // `none` only when it is not alive; the owner as heir stays the owner
    // (`erl_db.c:5319-5337`).
    made.set_heir(heir.filter(|heir| *heir == owner));
    let name_text = name.map(|name| context.scheduler.atoms.name_of(name));
    let mut ets = context.scheduler.ets.exclusive();
    // The name check and the create take one lock, so no other scheduler
    // takes the name between them, as `insert_named_tab`
    // (`erl_db.c:1000-1047`) checks the name after `make_btid`
    // (`erl_db.c:2811-2831`).
    if name_text.is_some_and(|name| ets.name_in_use(name)) {
        return Ok(denied("already_exists"));
    }
    if let Some(heir) = heir
        && heir != owner
        && (context.scheduler.process(heir).is_some()
            || context.scheduler.remote_owner(heir).is_some())
    {
        // ponytail: no table transfer to a heir; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!("unsupported ets:new/2 heir {heir:?}")));
    }
    ets.create(table, name_text, made);
    drop(ets);
    Ok(Value(name.map_or(Term::Reference(table), |name| {
        Term::Atom(name.clone())
    })))
}

/// Run `ets:insert(Table, Objects)` and return `true`, as ERTS
/// `ets_insert_2` (`erl_db.c:2341-2381`) does. `Objects` is one tuple or a
/// list of tuples, and each object goes into the table as [`Table::insert`]
/// puts it, also after an earlier object of the same list.
///
/// One tuple and a list of one tuple cost one reduction for each 8 objects
/// that a `bag` insert compares, and no reduction in another table
/// (`erl_db.c:2375-2379`, `erl_db_hash.c:1422-1442`). `[]` costs no
/// reduction (`erl_db.c:2349-2353`). A table that does not exist or that the
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
/// let object = Term::tuple(vec![test_atom("k"), test_atom("v")]);
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
    insert_objects(context, table, objects, false)
}

/// Run `ets:insert_new(Table, Objects)`, as ERTS `ets_insert_new_2`
/// (`erl_db.c:2387-2440`) does. It costs, checks, and traps as [`insert`]
/// does, with one difference: when the table holds the key of an object,
/// no object goes into the table, and the call returns `false`. One tuple
/// and a list of one tuple then cost no reduction: `db_put_hash` with
/// `key_clash_fail` stops before it compares an object
/// (`erl_db_hash.c:1396-1428`). The table holds only the objects of
/// earlier calls, so a key that only the list holds two times is no clash
/// (`ets_insert_new_2_list_has_member`, `erl_db.c:1829-1842`), and the
/// list goes into the table as `ets:insert/2` puts it. [`insert_list`]
/// gives the units of a longer list.
///
/// # Examples
///
/// ```rust,ignore
/// let object = Term::tuple(vec![test_atom("k"), test_atom("v")]);
/// assert!(matches!(insert_new(&mut context, &table, &object)?, BifResult::Value(_)));
/// // `false`: the table holds the key `k`.
/// assert!(matches!(insert_new(&mut context, &table, &object)?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// Return the runtime errors of [`insert`].
pub(super) fn insert_new(
    context: &mut BifContext<'_>,
    table: &Term,
    objects: &Term,
) -> Result<BifResult, BeamError> {
    insert_objects(context, table, objects, true)
}

/// Run [`insert`], or [`insert_new`] when `new` is true.
fn insert_objects(
    context: &mut BifContext<'_>,
    table: &Term,
    objects: &Term,
    new: bool,
) -> Result<BifResult, BeamError> {
    // A cons cell with a tail that is not `[]`: at least two objects, or an
    // improper list.
    if objects.cons_cell().is_some_and(|(_, tail)| !tail.is_nil()) {
        let store = Arc::clone(&context.scheduler.ets);
        let ets = store.shared();
        let Some((id, found)) = ets.find(&context.scheduler.atoms, table) else {
            return Ok(denied(EtsTables::missing(table)));
        };
        let (keypos, bag) = (found.keypos(), found.table_type() == TableType::Bag);
        drop(found);
        drop(ets);
        let state = ListState {
            done: 0,
            bag: bag.then(Term::nil),
            check: Check::Rest(objects.clone()),
            new: if new { New::Unlocked } else { New::No },
            refused: None,
        };
        return insert_list(context, id, objects, keypos, state);
    }
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let mut table = match ets.write(&context.scheduler.atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            return Ok(retry(
                context,
                lacking,
                if new { "insert_new" } else { "insert" },
                vec![table.clone(), objects.clone()],
            ));
        }
    };
    let objects: Vec<&Term> = match objects.proper_list() {
        Some(objects) => objects.collect(),
        None => vec![objects],
    };
    let keypos = table.keypos();
    let Some(tuples) = valid_objects(objects, keypos) else {
        return Ok(BifResult::badarg());
    };
    if new && first_held(&context.scheduler.atoms, &table, &tuples, keypos)?.is_some() {
        return Ok(Value(Term::static_atom("false")));
    }
    let compared = put_objects(&context.scheduler.atoms, &mut table, &tuples, keypos, new)?;
    drop(table);
    drop(ets);
    let compared: usize = compared.iter().sum();
    // `consumed_reds / ITERATIONS_PER_RED` (`erl_db.c:2379`).
    let reductions = i32::try_from(compared / 8).unwrap_or(i32::MAX);
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(reductions);
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
/// 2. When the table is not a `bag` and the units that remain are at least
///    the number of objects (`can_insert_without_yield`,
///    `erl_db.c:2002-2014`), the caller must
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
/// In a `bag` table, the insert of each object uses one more unit for each
/// object that it compares (`erl_db.c:1934-1940`, `erl_db_hash.c:1422-1442`)
/// in one step, and the YCF counter yields only after a step: first 1 unit
/// for the loop, then `1 + n` units for the insert of an object that
/// compares `n` objects. The `input_probe` cost of a 400-object `bag` insert
/// matches ERTS only in this order. `bag` is `None` for another table, and else the list
/// of the steps of the insert that remain: `[]` before the insert and after
/// a failed copy.
///
/// The call bumps one reduction for each 8 units that it used
/// (`erl_db.c:2197`, `2306`). When the unit that uses the last of the
/// budget, or the first unit for a budget of 0, is not the last unit, the
/// call uses all reductions and traps to its export with `done` after that
/// unit (`erl_db.c:2297-2332`). ERTS traps to `ets:insert/2` with a magic
/// reference to the YCF state, and PBeam traps to the internal export
/// `ets:insert/6` with the table reference, the objects, `done`, the key
/// position, the steps of a `bag` insert, or `false` for another table, and
/// the [`Check`]. A resume reads only the part of the list that the check
/// did not read, and after the check it does not read the list again
/// before the insert. `done` counts the units of the check, the copy, and the
/// insert, and after a failed copy the units of the removal from
/// `4 * count`. ERTS takes a table lock and walks and inserts the objects
/// of step 3 one after another, and the table is busy between the objects,
/// so another process that uses it completes the walk and the insert first
/// and charges its own units for them ([`crate::runtime::ets::EtsTables::read`]). When the owner exit
/// deletes the table during step 3, a later call raises `badarg`, as ERTS
/// does after it cancels the insert (`erl_db.c:2232-2248`, `5082-5086`). `ets:delete/1` in ERTS first
/// completes the insert, so that the insert returns `true`, and PBeam
/// returns a runtime error there ([`delete_table`]).
///
/// `new` is [`New::No`] for `ets:insert/2`. For `ets:insert_new/2`
/// (`erl_db.c:2017-2116`), the call walks the objects up to the first
/// object whose key the table holds, with one unit for each object
/// (`ets_insert_new_2_list_has_member`, `erl_db.c:1829-1842`, `1909-1925`),
/// after it takes the table and before the insert. When it finds such an
/// object, no object goes into the table, and the call returns `false`: in
/// step 2 with no more units, and in step 3 after it frees the copies with
/// one unit for each object (`erl_db.c:2091-2098`). In step 3 PBeam decides
/// the result when it takes the table, and keeps the units of the walk in
/// [`New::Locked`]. The call then traps to the internal export
/// `ets:insert_new/7`, with the state of `new` after the arguments of
/// `ets:insert/6`. ERTS traps to `ets:insert_new/2` (`erl_db.c:2211`).
///
/// # Examples
///
/// ```rust,ignore
/// process.fcalls = 10;
/// let objects = Term::list_from(vec![Term::tuple(vec![test_atom("k")]); 8]);
/// // 8 units of the check and 16 units of the insert: 3 reductions.
/// let state = ListState { done: 0, bag: None, check: Check::Rest(objects.clone()), new: New::No, refused: None };
/// assert!(matches!(insert_list(&mut context, id, &objects, 1, state)?, BifResult::Value(_)));
/// assert_eq!(context.scheduler.current_process_mut().fcalls, 7);
/// ```
///
/// # Errors
///
/// Return a runtime error for a table with the key position 0: ERTS aborts
/// the emulator there (`Invalid tag in make_internal_hash`).
pub(super) fn insert_list(
    context: &mut BifContext<'_>,
    id: Reference,
    objects: &Term,
    keypos: i32,
    state: ListState,
) -> Result<BifResult, BeamError> {
    let ListState {
        mut done,
        mut bag,
        mut check,
        mut new,
        mut refused,
    } = state;
    let table = Term::Reference(id);
    let inserting = |bag: &Option<Term>| bag.as_ref().is_some_and(|steps| !steps.is_nil());
    // The number of objects is known only after the check. Until then,
    // `done` takes its place, so that each test of a later phase is false.
    let mut count = match check {
        Check::Rest(_) => done,
        Check::Done(count) => count,
    };
    let locked = matches!(new, New::Locked { .. });
    // Step 3 of the call is the walk of `ets:insert_new/2` and the insert
    // of the objects, one step for each object (`erl_db.c:2072-2110`). A
    // plain `ets:insert/2` marks it with the units that the copies and the
    // insert of the objects used, a `bag` with the steps that remain, and
    // `ets:insert_new/2` with [`New::Locked`].
    let global = locked
        || match &bag {
            Some(_) => inserting(&bag),
            None => 2 * count < done && done <= 4 * count,
        };
    if global {
        // The insert ended in another turn: the process reads its result,
        // as the driver reads `ctx->return_value`, also after the table
        // went away (`erl_db.c:2223-2244`).
        // The check of the table goes first: an insert that another
        // scheduler completes keeps its result before the table can go.
        let caller = context.scheduler.current_process_mut().id;
        let ets = context.scheduler.ets.shared();
        let gone = ets.find(&context.scheduler.atoms, &table).is_none();
        if let Some(result) = ets.take_result(caller) {
            return Ok(Value(result));
        }
        drop(ets);
        if gone {
            // The table went away: ERTS raises with the cause `id` of the
            // deleted table (`erl_db.c:2232-2247`).
            return Ok(denied("id"));
        }
    }
    let process = context.scheduler.current_process_mut();
    let caller = process.id;
    // A `long` count, and YCF yields when it is 0 or less (`erl_db.c:2258`,
    // `ycf_yield_fun.c:592-610`): a negative budget does what 0 does.
    let budget = ITERATIONS_PER_RED * process.bif_reductions_or_zero() as i64;
    let mut units = Units { left: budget };
    // The tuples of the checked list, for the insert. The walk is part of
    // the units of the insert.
    let tuples = || -> Vec<&[Term]> {
        (objects.list_iter().expect("a list insert has a list"))
            .map(|object| valid_object(object, keypos).expect("the check found each object good"))
            .collect()
    };
    // `None` when the call yields, else the result of the call.
    let result = 'work: {
        if let Check::Rest(rest) = &mut check {
            match units.check(&mut done, rest, keypos) {
                None => break 'work None,
                Some(false) => {
                    // ERTS takes the table before it raises `badarg` for a
                    // bad object, so a busy table runs its list insert first
                    // (`erl_db.c:2036-2047`).
                    // The help uses the budget of the process, as
                    // `ERTS_BIF_REDS_LEFT` gives it (`erl_db.c:2152-2158`).
                    let reds = context
                        .scheduler
                        .current_process_mut()
                        .bif_reductions_signed();
                    let ets = context.scheduler.ets.shared();
                    let taken = ets.write(&context.scheduler.atoms, &table, caller, reds);
                    match taken {
                        Err(Lacking::Busy { reductions }) => {
                            units.left -= reductions * ITERATIONS_PER_RED;
                            break 'work None;
                        }
                        // The denial keeps its cause (`erl_db.c:2040-2048`).
                        Err(Lacking::Denied(cause)) => break 'work Some(denied(cause)),
                        Ok(_) => break 'work Some(BifResult::badarg()),
                    }
                }
                Some(true) => (count, check) = (done, Check::Done(done)),
            }
        }
        // `can_insert_without_yield` (`erl_db.c:2002-2014`): the check is
        // done, and a table that is not a `bag` whose objects the units
        // cover takes them with no copy and no trap (`erl_db.c:2050-2071`).
        // A call that traps in the take of the table then runs the same
        // branch in its next turn (`erl_db.c:1963-1994`).
        if matches!(check, Check::Done(_))
            && bag.is_none()
            && done == count
            && count as i64 <= units.left
            && !locked
        {
            // A second call completes the list insert that holds the
            // table busy before it takes it, also without a copy of the
            // objects (`erl_db.c:2050-2055`).
            // The help uses the budget of the process, as
            // `ERTS_BIF_REDS_LEFT` gives it (`erl_db.c:2152-2158`).
            let reds = context
                .scheduler
                .current_process_mut()
                .bif_reductions_signed();
            let ets = context.scheduler.ets.shared();
            let mut table = match ets.write(&context.scheduler.atoms, &table, caller, reds) {
                Ok(table) => table,
                Err(Lacking::Denied(cause)) => break 'work Some(denied(cause)),
                Err(Lacking::Busy { reductions }) => {
                    units.left -= reductions * ITERATIONS_PER_RED;
                    break 'work None;
                }
            };
            let tuples = tuples();
            if !matches!(new, New::No) {
                let held = first_held(&context.scheduler.atoms, &table, &tuples, keypos)?;
                units.left -= held.unwrap_or(count) as i64;
                if held.is_some() {
                    break 'work Some(Value(Term::static_atom("false")));
                }
            }
            // The list call does its own membership walk and passes `false`
            // to `db_put` (`erl_db.c:1858`, `1943`).
            put_objects(&context.scheduler.atoms, &mut table, &tuples, keypos, false)?;
            units.left -= 2 * count as i64;
            break 'work Some(Value(Term::static_atom("true")));
        }
        let mut took = false;
        if done <= 2 * count && !inserting(&bag) && !locked {
            if !units.run(&mut done, 2 * count) {
                break 'work None;
            }
            // The copies are done: the table takes the walk of
            // `ets:insert_new/2` and the insert of the objects
            // (`erl_db.c:2072-2110`), and it keeps the copies and the
            // position of the steps, so a call that traps between two
            // objects leaves it busy (`erl_db.c:2310-2319`). A list insert
            // of another process that did not finish first runs here, with
            // the units of this call, and this call then takes the table in
            // a later turn (`handle_lacking_permission`, `erl_db.c:769-798`,
            // `2223-2253`).
            // The help uses the budget of the process, as
            // `ERTS_BIF_REDS_LEFT` gives it (`erl_db.c:2152-2158`).
            let reds = context
                .scheduler
                .current_process_mut()
                .bif_reductions_signed();
            let ets = context.scheduler.ets.shared();
            match ets.write(&context.scheduler.atoms, &table, caller, reds) {
                Err(Lacking::Busy { reductions }) => {
                    units.left -= reductions * ITERATIONS_PER_RED;
                    break 'work None;
                }
                Ok(mut taken_table) => {
                    let tuples = tuples();
                    let walk = matches!(new, New::Unlocked);
                    let bif = if walk { "insert_new/2" } else { "insert/2" };
                    check_keypos(&tuples, keypos, bif)?;
                    ets.reserve_result(caller);
                    taken_table.busy = Some(Busy::new(
                        caller,
                        tuples.iter().map(|object| object.to_vec()).collect(),
                        walk,
                    ));
                    // The call keeps its own BIF: `ets:insert_new/2` marks
                    // the steps of the walk and the insert with
                    // [`New::Locked`], and `ets:insert/2` with the units
                    // that its copy and its steps used.
                    if matches!(new, New::Unlocked) {
                        new = New::Locked {
                            walk: 0,
                            found: false,
                        };
                    }
                    if bag.is_some() {
                        bag = Some(Term::list_from([Term::Integer(0)]));
                    }
                    took = true;
                }
                Err(Lacking::Denied(cause)) => {
                    done = 4 * count;
                    refused = Some(cause);
                }
            }
        }
        if took || global {
            // The steps of the insert that the table took: the walk and
            // the insert of the objects, one step for each object
            // (`erl_db.c:1927-1949`, `2091-2107`). The call traps between
            // the objects, and its next turn runs the steps that remain.
            let (used, _) = step_inserting(&mut *context, &table, units.left, Some(caller))?;
            done += used as usize;
            units.left -= used;
            match context.scheduler.ets.shared().take_result(caller) {
                Some(value) => break 'work Some(Value(value)),
                None => break 'work None,
            }
        }
        if !units.run(&mut done, 5 * count) {
            break 'work None;
        }
        // The call keeps the cause of the refusal across the yields of the
        // free (`erl_db.c:2076-2088`).
        Some(denied(
            refused.expect("only a refused insert frees its copies"),
        ))
    };
    let process = context.scheduler.current_process_mut();
    process.bump_reductions(((budget - units.left) / ITERATIONS_PER_RED) as i32);
    Ok(result.unwrap_or_else(|| {
        let mut arguments = vec![
            table,
            objects.clone(),
            Term::Integer(done as i64),
            Term::Integer(i64::from(keypos)),
            // A refused insert keeps no `bag` steps, and the slot keeps the
            // cause of the refusal.
            match refused {
                Some(cause) => Term::static_atom(cause),
                None => bag.unwrap_or_else(|| Term::static_atom("false")),
            },
            check.into_term(),
        ];
        let function = match new.into_term() {
            Some(new) => {
                arguments.push(new);
                "insert_new"
            }
            None => "insert",
        };
        BifResult::TrapBif {
            module: "ets",
            function,
            arguments,
        }
    }))
}

/// The state of a list insert ([`insert_list`]) that a trap keeps.
pub(super) struct ListState {
    /// The units that the call used.
    pub(super) done: usize,
    /// The steps of a `bag` insert that remain, or `None` for another
    /// table.
    pub(super) bag: Option<Term>,
    /// How far the check went.
    pub(super) check: Check,
    /// The `ets:insert_new/2` state.
    pub(super) new: New,
    /// The `error_info` cause of the table that refused the insert, which
    /// the call raises after it frees its copies (`erl_db.c:2076-2088`).
    pub(super) refused: Option<&'static str>,
}

/// The `ets:insert_new/2` state of a list insert ([`insert_list`]).
pub(super) enum New {
    /// The call is `ets:insert/2`.
    No,
    /// The call is `ets:insert_new/2`, and it did not take the table.
    Unlocked,
    /// The call is `ets:insert_new/2`, and it took the table. `walk` is the
    /// number of units of the walk for a key that the table holds, and
    /// `found` is true when the walk found one.
    Locked { walk: usize, found: bool },
}

impl New {
    /// Return the trap argument of the state: `true` before the call takes
    /// the table, `{Walk, Found}` after, and `None` for `ets:insert/2`.
    /// [`New::from_term`] reads it back.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(New::Unlocked.into_term(), Some(Term::static_atom("true")));
    /// assert!(New::No.into_term().is_none());
    /// ```
    fn into_term(self) -> Option<Term> {
        match self {
            New::No => None,
            New::Unlocked => Some(Term::static_atom("true")),
            New::Locked { walk, found } => Some(Term::tuple(vec![
                Term::Integer(walk as i64),
                Term::static_atom(if found { "true" } else { "false" }),
            ])),
        }
    }

    /// Read the trap argument that [`New::into_term`] gives.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(New::from_term(&Term::static_atom("true")), New::Unlocked));
    /// ```
    pub(super) fn from_term(term: &Term) -> New {
        match term {
            Term::Tuple(elements) => New::Locked {
                walk: elements[0].small_integer().expect("a trap keeps the walk") as usize,
                found: elements[1] == Term::static_atom("true"),
            },
            _ => New::Unlocked,
        }
    }
}

/// How far the check of a list insert ([`insert_list`]) went, as the YCF
/// state of `ets_insert_2_list_check` keeps the list cell that it reads
/// (`erl_db.c:1812-1827`). A resume continues from there, so no turn walks
/// the checked objects again.
pub(super) enum Check {
    /// The check runs. The term is the part of the list that it did not
    /// read: a cons cell, `[]` after the last object, or another term after
    /// a bad object or as the tail of an improper list, which raises
    /// `badarg`.
    Rest(Term),
    /// The check found each object good. The number is the number of
    /// objects.
    Done(usize),
}

impl Check {
    /// Return the trap argument of the check: `{Rest}` or the number of
    /// objects. [`Check::from_term`] reads it back.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Check::Done(2).into_term(), Term::Integer(2));
    /// assert_eq!(Check::Rest(Term::nil()).into_term(), Term::tuple(vec![Term::nil()]));
    /// ```
    fn into_term(self) -> Term {
        match self {
            Check::Rest(rest) => Term::tuple(vec![rest]),
            Check::Done(count) => Term::Integer(count as i64),
        }
    }

    /// Read the trap argument that [`Check::into_term`] gives.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(matches!(Check::from_term(&Term::Integer(2)), Check::Done(2)));
    /// assert!(matches!(Check::from_term(&Term::tuple(vec![Term::nil()])), Check::Rest(_)));
    /// ```
    pub(super) fn from_term(term: &Term) -> Check {
        match term {
            Term::Tuple(elements) => Check::Rest(elements[0].clone()),
            count => Check::Done(count.small_integer().expect("a trap keeps the count") as usize),
        }
    }
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

    /// Check the objects of the list `rest` with one unit for each object,
    /// up to the first bad object, as `ets_insert_2_list_check`
    /// (`erl_db.c:1812-1827`) does, and as [`Units::run`] counts the units.
    /// Read at most the objects that the units allow. Return `Some(true)`
    /// when each object is good and the list is proper, `Some(false)` for a
    /// bad object or an improper tail, and `None` when the call yields:
    /// `rest` then keeps the part of the list that remains.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut units, mut done) = (Units { left: 1 }, 0);
    /// let mut rest = Term::list_from(vec![Term::tuple(vec![test_atom("k")]); 2]);
    /// assert_eq!(units.check(&mut done, &mut rest, 1), None);
    /// assert_eq!((done, units.left), (1, 0));
    /// ```
    fn check(&mut self, done: &mut usize, rest: &mut Term, keypos: i32) -> Option<bool> {
        let limit = self.left.max(1);
        let mut walked = 0;
        let good = loop {
            let Some((object, tail)) = rest.cons_cell() else {
                break rest.is_nil();
            };
            if walked == limit {
                break true;
            }
            walked += 1;
            let good = valid_object(object, keypos).is_some();
            // A term that is not a list keeps the bad object for a resume.
            *rest = if good {
                tail.clone()
            } else {
                Term::static_atom("badarg")
            };
            if !good {
                break false;
            }
        };
        *done += walked as usize;
        if walked >= limit {
            self.left -= limit;
            return None;
        }
        self.left -= walked;
        Some(good)
    }
}

/// Put each object of `tuples` into `table`, which has the key position
/// `keypos`, and return the number of objects that the insert of each
/// object compared ([`Table::insert_all`]). `atoms` owns the name of an atom key
/// of an object.
///
/// # Examples
///
/// ```rust,ignore
/// let object = vec![test_atom("k")];
/// assert_eq!(put_objects(&atoms, &mut table, &[&object], 1)?, vec![0]);
/// assert!(table.object(&atoms, &test_atom("k")).is_some());
/// ```
///
/// # Errors
///
/// Return a runtime error for an object in a table with the key position
/// 0: ERTS aborts the emulator there (`Invalid tag in make_internal_hash`).
fn put_objects(
    atoms: &AtomTable,
    table: &mut Table,
    tuples: &[&[Term]],
    keypos: i32,
    new: bool,
) -> Result<Vec<usize>, BeamError> {
    check_keypos(tuples, keypos, "insert/2")?;
    let objects = tuples.iter().map(|elements| elements.to_vec()).collect();
    Ok(if new {
        table.insert_all_new(atoms, objects)
    } else {
        table.insert_all(atoms, objects)
    })
}

/// Return the position, from 1, of the first object of `tuples` whose key
/// `table` holds, or `None`, as `ets_insert_new_2_list_has_member`
/// (`erl_db.c:1829-1842`, `1909-1925`) looks for it. The table has the key
/// position `keypos`, and `atoms` owns the name of an atom key.
///
/// # Examples
///
/// ```rust,ignore
/// let object = vec![test_atom("k")];
/// put_objects(&atoms, &mut table, &[&object], 1)?;
/// assert_eq!(first_held(&atoms, &table, &[&[test_atom("j")], &object], 1)?, Some(2));
/// ```
///
/// # Errors
///
/// Return the runtime error of [`check_keypos`].
fn first_held(
    atoms: &AtomTable,
    table: &Table,
    tuples: &[&[Term]],
    keypos: i32,
) -> Result<Option<usize>, BeamError> {
    check_keypos(tuples, keypos, "insert_new/2")?;
    // The check makes the key position at least 1 for each object.
    let held = (tuples.iter()).position(|elements| {
        table
            .object(atoms, &elements[keypos as usize - 1])
            .is_some()
    });
    Ok(held.map(|index| index + 1))
}

/// Return a runtime error for the objects `tuples` of the BIF `bif` in a
/// table with the key position 0: ERTS aborts the emulator there (`Invalid
/// tag in make_internal_hash`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(check_keypos(&[], 0, "insert/2").is_ok());
/// assert!(check_keypos(&[&[test_atom("k")]], 0, "insert/2").is_err());
/// ```
///
/// # Errors
///
/// Return the runtime error for a table with the key position 0.
fn check_keypos(tuples: &[&[Term]], keypos: i32, bif: &str) -> Result<(), BeamError> {
    if keypos == 0 && !tuples.is_empty() {
        // ponytail: ERTS aborts here; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!(
            "unsupported ets:{bif} in a table with key position 0"
        )));
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
/// let object = Term::tuple(vec![test_atom("k")]);
/// assert!(valid_objects([&object], 1).is_some());
/// assert!(valid_objects([&object], 2).is_none());
/// assert!(valid_objects([&object], -1).is_none());
/// ```
fn valid_objects<'a>(
    objects: impl IntoIterator<Item = &'a Term>,
    keypos: i32,
) -> Option<Vec<&'a [Term]>> {
    objects
        .into_iter()
        .map(|object| valid_object(object, keypos))
        .collect()
}

/// Return the elements of `object` when it is a tuple with at least
/// `keypos` elements, as [`valid_objects`] checks each object.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(valid_object(&Term::tuple(vec![test_atom("k")]), 1).is_some());
/// assert!(valid_object(&test_atom("k"), 1).is_none());
/// ```
fn valid_object(object: &Term, keypos: i32) -> Option<&[Term]> {
    match object {
        Term::Tuple(elements) if elements.len() as u64 >= i64::from(keypos) as u64 => {
            Some(elements.as_ref())
        }
        _ => None,
    }
}

/// Run `ets:lookup_element(Table, Key, Index)` or
/// `ets:lookup_element(Table, Key, Index, Default)`, as ERTS
/// `ets_lookup_element_3` and `ets_lookup_element_4` (`erl_db.c:2935-3001`)
/// and `db_get_element_hash` (`erl_db_hash.c:1544-1608`) do: return the
/// element at `Index` of the object with the key `Key` in a `set` table, the
/// list of the elements at `Index` of the objects with the key in insertion
/// order in another table, or `Default` when the table has no object with
/// that key. Without a `Default`, a missing key raises `badarg` with the
/// cause `badkey` (`erl_db.c:2959-2961`).
///
/// A table that does not exist or that the caller cannot read raises
/// `badarg` first. Then an index that is not a small integer or that is
/// below 1 raises `badarg`. ERTS gives the index to `db_get_element` as a C
/// `int` (`erl_db_util.h:151-155`), so only its low 32 bits count. For a
/// found key, a negative position, or a position above the size of one of
/// its objects, raises `badarg` (`erl_db_hash.c:1563-1580`).
///
/// # Examples
///
/// ```rust,ignore
/// let key = test_atom("k");
/// let result = lookup_element(&mut context, &table, &key, &Term::Integer(2), Some(&test_atom("d")))?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("v")));
/// let result = lookup_element(&mut context, &table, &key, &Term::Integer(0), None)?;
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
    default: Option<&Term>,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let table = match ets.read(&context.scheduler.atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            let mut arguments = vec![table.clone(), key.clone(), index.clone()];
            arguments.extend(default.cloned());
            return Ok(retry(context, lacking, "lookup_element", arguments));
        }
    };
    let Some(index) = index.small_integer().filter(|index| *index >= 1) else {
        return Ok(BifResult::badarg());
    };
    let objects = table.objects(&context.scheduler.atoms, key);
    if objects.is_empty() {
        return Ok(match default {
            Some(default) => Value(default.clone()),
            None => BifResult::ErrorWithCause {
                reason: Term::static_atom("badarg"),
                cause: Term::static_atom("badkey"),
            },
        });
    }
    let position = index as i32;
    if position == 0 {
        // ponytail: ERTS crashes here; see "ETS tables" in docs/erts-differences.md.
        return Err(error(format!(
            "unsupported ets:lookup_element/4 index {index} with position 0"
        )));
    }
    let elements: Option<Vec<Term>> = objects
        .iter()
        .map(|object| {
            let position = usize::try_from(position).ok()?;
            object.get(position - 1).cloned()
        })
        .collect();
    Ok(match (elements, table.table_type()) {
        (None, _) => BifResult::badarg(),
        (Some(mut elements), TableType::Set | TableType::OrderedSet) => Value(elements.remove(0)),
        (Some(elements), _) => Value(Term::list_from(elements)),
    })
}

/// Run `ets:lookup(Table, Key)`, as ERTS `ets_lookup_2`
/// (`erl_db.c:2877-2900`) and `db_get_hash` (`erl_db_hash.c:1469-1516`) do:
/// return the list of the objects with a key that is exactly equal to
/// `Key`, in insertion order, or `[]`. A `set` table has at most one. A table that does not exist or that the caller cannot
/// read raises `badarg` (`erl_db.c:769-912`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = lookup(&mut context, &table, &test_atom("k"));
/// assert!(matches!(result, BifResult::Value(objects) if objects.proper_list().is_some_and(|objects| objects.count() == 1)));
/// assert!(matches!(lookup(&mut context, &test_atom("missing"), &test_atom("k")), BifResult::Raise { .. }));
/// ```
pub(super) fn lookup(
    context: &mut BifContext<'_>,
    table: &Term,
    key: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let table = match ets.read(&context.scheduler.atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            return Ok(retry(
                context,
                lacking,
                "lookup",
                vec![table.clone(), key.clone()],
            ));
        }
    };
    let objects = table.objects(&context.scheduler.atoms, key);
    Ok(Value(Term::list_from(
        objects
            .into_iter()
            .map(|object| Term::tuple(object.to_vec())),
    )))
}

/// Run `ets:member(Table, Key)`, as ERTS `ets_member_2`
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
pub(super) fn member(
    context: &mut BifContext<'_>,
    table: &Term,
    key: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let table = match ets.read(&context.scheduler.atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            return Ok(retry(
                context,
                lacking,
                "member",
                vec![table.clone(), key.clone()],
            ));
        }
    };
    let found = table.object(&context.scheduler.atoms, key).is_some();
    Ok(Value(Term::static_atom(if found {
        "true"
    } else {
        "false"
    })))
}

/// Run `ets:delete(Table, Key)` and return `true`, as ERTS `ets_delete_2`
/// (`erl_db.c:3328-3349`) and `db_erase_hash` (`erl_db_hash.c:1613-1664`)
/// do. Each object with a key that is exactly equal to `Key` goes out of the
/// table. A key that the table does not hold
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
pub(super) fn delete(
    context: &mut BifContext<'_>,
    table: &Term,
    key: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let mut table = match ets.write(&context.scheduler.atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            return Ok(retry(
                context,
                lacking,
                "delete",
                vec![table.clone(), key.clone()],
            ));
        }
    };
    table.delete(&context.scheduler.atoms, key);
    Ok(Value(Term::static_atom("true")))
}

/// Run `ets:take(Table, Key)`, as ERTS `ets_take_2` (`erl_db.c:1355-1373`)
/// does through `meth->db_take`, and `db_take_hash`
/// (`erl_db_hash.c:2949-2995`) does for a hash table: return the objects with
/// a key that is exactly equal to `Key`, in the order of
/// [`lookup`], and remove them from the table. A key that the table does not
/// hold returns `[]`. A table that does not exist or that the caller cannot
/// write raises `badarg` (`erl_db.c:769-912`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = take(&mut context, &table, &test_atom("k"))?;
/// assert!(matches!(result, BifResult::Value(objects) if objects.proper_list().is_some_and(|objects| objects.count() == 1)));
/// ```
pub(super) fn take(
    context: &mut BifContext<'_>,
    table: &Term,
    key: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let mut table = match ets.write(&context.scheduler.atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            return Ok(retry(
                context,
                lacking,
                "take",
                vec![table.clone(), key.clone()],
            ));
        }
    };
    let objects: Vec<Term> = table
        .objects(&context.scheduler.atoms, key)
        .into_iter()
        .map(|object| Term::tuple(object.to_vec()))
        .collect();
    table.take_key(&context.scheduler.atoms, key);
    Ok(Value(Term::list_from(objects)))
}

/// Run `ets:delete(Table)`, as ERTS `ets_delete_1` (`erl_db.c:3006-3082`)
/// does:
///
/// - A table that does not exist or that the caller cannot write raises
///   `badarg` (`DB_WRITE`, `erl_db.c:3021`). Each process can delete a
///   `public` table, and only the owner can delete another table.
/// - The table loses its name and its reference at once, so no process
///   finds it again (`erl_db.c:3053-3061`). The delete disables the heir,
///   so no process gets the table or an `ETS-TRANSFER` message
///   (`erl_db.c:3056-3058`). PBeam has no table fixation to free.
/// - Then the caller frees the segments as [`delete_trap`] does.
///
/// # Examples
///
/// ```rust,ignore
/// let result = delete_table(&mut context, &table)?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("true")));
/// assert!(matches!(delete_table(&mut context, &table)?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// Return a runtime error when a list insert into the table traps: ERTS
/// first completes the insert, so that the insert returns `true`
/// (`erl_db.c:2232-2244`, `3021-3029`), and PBeam does not do this help.
/// Also return the errors of [`delete_trap`].
pub(super) fn delete_table(
    context: &mut BifContext<'_>,
    table: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let scheduler = &mut *context.scheduler;
    let store = Arc::clone(&scheduler.ets);
    let mut ets = store.exclusive();
    if let Err(lacking) = ets.write(&scheduler.atoms, table, caller, reds) {
        return Ok(retry(context, lacking, "delete", vec![table.clone()]));
    }
    let id = (ets.id(&scheduler.atoms, table)).expect("the caller found the table");
    if scheduler.traps_ets_insert(id) {
        return Err(error(
            "unsupported ets:delete/1 of a table that a trapped insert uses: see \"ETS tables\" in docs/erts-differences.md",
        ));
    }
    let fixations = ets.delete_table(id, caller);
    drop(ets);
    scheduler.schedule_pending_cleanups();
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(i32::try_from(fixations).unwrap_or(i32::MAX));
    delete_trap(context)
}

/// Free the segments of the table that `ets:delete/1` deleted, as
/// `free_table_continue` and `ets_delete_trap` (`erl_db.c:5378-5426`) do.
/// The work starts with the reductions that are left in the turn
/// (`ERTS_BIF_REDS_LEFT`). Each segment, from the top segment down, uses 32
/// reductions and 1 for each object in it (`erl_db_hash.c:3140-3149`). When
/// the reductions are less than 0 after a segment, the call uses all
/// reductions and traps to the internal export `ets:delete_trap/0`, which
/// continues in the next turn (`erl_db.c:3064-3077`). Else the call uses
/// the reductions of the segments and returns `true`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = delete_trap(&mut context)?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("true")));
/// ```
/// # Errors
///
/// The invariant error of [`ProcessState::bif_reductions_left`]: the call starts
/// at the start of a turn, or after [`delete_table`] bumps the reductions
/// of the fixations, which keeps the budget at 0 or more.
pub(super) fn delete_trap(context: &mut BifContext<'_>) -> Result<BifResult, BeamError> {
    let process = context.scheduler.current_process_mut();
    let caller = process.id;
    let left = i32::try_from(process.bif_reductions_left()?).unwrap_or(i32::MAX);
    let Some(rest) = (context.scheduler.ets.exclusive()).free_segments(caller, left) else {
        context
            .scheduler
            .current_process_mut()
            .bump_all_reductions();
        return Ok(BifResult::TrapBif {
            module: "ets",
            function: "delete_trap",
            arguments: Vec::new(),
        });
    };
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(left - rest);
    Ok(Value(Term::static_atom("true")))
}

/// Run `ets:info(Table, Item)`, as ERTS `ets_info_2` (`erl_db.c:4523-4579`)
/// does with `table_info` (`erl_db.c:5447-5625`). The item `binary` traps
/// before the table lookup (`erl_db.c:4541-4542`). Then the lookup asks only
/// for `DB_INFO` (`erl_db_util.h:451`), which each process has. A free name
/// gives `undefined` (`erl_db.c:4544-4547`), and a term that is not an atom
/// or a reference raises `badarg`. After the lookup, an item that
/// `table_info` does not know raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = info(&mut context, &table, &test_atom("type"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("set")));
/// let result = info(&mut context, &test_atom("missing"), &test_atom("type"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("undefined")));
/// assert!(matches!(info(&mut context, &table, &test_atom("size"))?, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// Return a runtime error for the items `binary`, `data`, and `stats`.
pub(super) fn info(
    context: &mut BifContext<'_>,
    table: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    let is_item = |name| matches!(item, Term::Atom(atom) if atom.is_static(name));
    if is_item("binary") {
        // ponytail: no binary reference list; see "ETS tables" in docs/erts-differences.md.
        return Err(error("unsupported ets:info/2 item binary"));
    }
    // `ets:info/2` asks for `DB_INFO`, so a busy table completes its list
    // insert first, and the call runs again in a later turn
    // (`erl_db.c:769-798`, `4549-4555`).
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let found = match ets.info(atoms, table, caller, reds) {
        Ok(found) => found,
        Err(lacking @ Lacking::Busy { .. }) => {
            return Ok(retry(
                context,
                lacking,
                "info",
                vec![table.clone(), item.clone()],
            ));
        }
        // A table that no longer exists gives `undefined`; another term
        // raises with the cause `type` (`erl_db.c:4434-4441`, `4544-4551`).
        Err(Lacking::Denied("id")) => return Ok(Value(Term::static_atom("undefined"))),
        Err(Lacking::Denied(cause)) => return Ok(denied(cause)),
    };
    let id = (ets.id(atoms, table)).expect("the caller found the table");
    let value = match item {
        Term::Atom(atom) if atom.is_static("type") => Term::static_atom(found.table_type().name()),
        Term::Atom(atom) if atom.is_static("size") => Term::from_bigint(BigInt::from(found.size())),
        Term::Atom(atom) if atom.is_static("memory") => Term::Integer(found.memory_words() as i64),
        Term::Atom(atom) if atom.is_static("owner") => Term::Pid(found.owner()),
        Term::Atom(atom) if atom.is_static("heir") => {
            found.heir().map_or(Term::static_atom("none"), Term::Pid)
        }
        Term::Atom(atom) if atom.is_static("protection") => {
            Term::static_atom(access_name(found.access()))
        }
        Term::Atom(atom) if atom.is_static("write_concurrency") => {
            found.write_concurrency().clone()
        }
        Term::Atom(atom) if atom.is_static("read_concurrency") => {
            boolean_term(found.read_concurrency())
        }
        Term::Atom(atom) if atom.is_static("name") => Term::Atom(
            atoms
                .get(found.name())
                .expect("the table name atom is in the table"),
        ),
        Term::Atom(atom) if atom.is_static("keypos") => Term::Integer(i64::from(found.keypos())),
        Term::Atom(atom) if atom.is_static("node") => Term::static_atom("nonode@nohost"),
        Term::Atom(atom) if atom.is_static("named_table") => boolean_term(ets.named(id)),
        Term::Atom(atom) if atom.is_static("compressed") => boolean_term(found.compressed()),
        Term::Atom(atom) if atom.is_static("id") => Term::Reference(id),
        Term::Atom(atom) if atom.is_static("decentralized_counters") => {
            boolean_term(found.decentralized_counters())
        }
        Term::Atom(atom) if atoms.name_of(atom) == "fixed" => boolean_term(found.is_fixed()),
        Term::Atom(atom)
            if matches!(
                atoms.name_of(atom),
                "safe_fixed" | "safe_fixed_monotonic_time"
            ) =>
        {
            if found.is_fixed() {
                // ponytail: the fixations of each process with their times
                // and counts; see "ETS tables" in docs/erts-differences.md.
                return Err(error(format!(
                    "unsupported ets:info/2 item {} of a fixed table",
                    atoms.name_of(atom)
                )));
            }
            Term::static_atom("false")
        }
        Term::Atom(atom) if matches!(atoms.name_of(atom), "data" | "stats") => {
            // ponytail: no ERTS debug print or chain statistics; see
            // "ETS tables" in docs/erts-differences.md.
            return Err(error(format!(
                "unsupported ets:info/2 item {}",
                atoms.name_of(atom)
            )));
        }
        _ => return Ok(BifResult::badarg()),
    };
    Ok(Value(value))
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
        if tag.is_static("debug_hash_fixed_number_of_locks") && LOCKS.contains(count))
}

/// Compile the match specification `spec`, as `db_match_compile` does.
/// Return `Ok(None)` when ERTS raises `badarg`, and a runtime error for a
/// guard function that PBeam does not run yet.
fn compile_spec(atoms: &AtomTable, spec: &Term) -> Result<Option<MatchSpec>, BeamError> {
    match MatchSpec::compile(atoms, spec) {
        Ok(compiled) => Ok(Some(compiled)),
        Err(SpecError::Badarg) => Ok(None),
        Err(SpecError::Unsupported(name)) => Err(error(format!(
            "unsupported ets:match specification function {name}: see \"ETS tables\" in docs/erts-differences.md"
        ))),
    }
}

/// Return the match specification of a pattern, as `ets:match/2,3` makes
/// it: `[{Pattern, [], ['$$']}]` (`erl_db.c:3781-3787`, `3809-3815`).
fn match_specification(pattern: &Term) -> Term {
    Term::list_from(vec![Term::tuple(vec![
        pattern.clone(),
        Term::nil(),
        Term::list_from(vec![Term::static_atom("$$")]),
    ])])
}

/// Return the match specification of a pattern, as `ets:match_object/2`
/// makes it: `[{Pattern, [], ['$_']}]` (`erl_db.c:4353-4359`).
fn match_object_specification(pattern: &Term) -> Term {
    Term::list_from(vec![Term::tuple(vec![
        pattern.clone(),
        Term::nil(),
        Term::list_from(vec![Term::static_atom("$_")]),
    ])])
}

/// The most objects that one turn of a match reads before it can trap
/// (`MAX_SELECT_CHUNK_ITERATIONS`, `erl_db_hash.c:2161`).
const MATCH_ITERATIONS: i64 = 1000;

/// Run `ets:match(Table, Pattern)` or, with `Some(Limit)`,
/// `ets:match(Table, Pattern, Limit)`, as `ets_match_2` and `ets_match_3`
/// (`erl_db.c:3771-3818`) do.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_objects(&mut context, &table, &test_atom("$1"), None)?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// See [`select_objects`].
pub(super) fn match_objects(
    context: &mut BifContext<'_>,
    table: &Term,
    pattern: &Term,
    limit: Option<&Term>,
) -> Result<BifResult, BeamError> {
    select_objects(context, table, &match_specification(pattern), limit, false)
}

/// Run `ets:match_object(Table, Pattern)`, as `ets_match_object_2`
/// (`erl_db.c:4343-4362`) does: [`select_objects`] with the specification
/// `[{Pattern, [], ['$_']}]`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_object(&mut context, &table, &test_atom("_"))?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// See [`select_objects`].
pub(super) fn match_object(
    context: &mut BifContext<'_>,
    table: &Term,
    pattern: &Term,
) -> Result<BifResult, BeamError> {
    select_objects(
        context,
        table,
        &match_object_specification(pattern),
        None,
        false,
    )
}

/// Run `ets:select(Table, MatchSpec)` or, with `Some(Limit)`,
/// `ets:select(Table, MatchSpec, Limit)`, as `ets_select_2` and
/// `ets_select_3` (`erl_db.c:3821-3835`, `4000-4048`) do:
///
/// 1. A table that does not exist or that the caller cannot read raises
///    `badarg` (`erl_db.c:769-912`).
/// 2. A limit that is not a small integer above 0 raises `badarg`
///    (`erl_db.c:3828-3832`).
/// 3. A match specification that does not compile raises `badarg`
///    (`erl_db.c:4016-4043`): see [`MatchSpec::compile`].
///
/// Then [`match_walk`] reads the table.
///
/// # Errors
///
/// Return a runtime error for a guard function that PBeam does not run
/// yet, and for a table that changed while the match trapped.
pub(super) fn select(
    context: &mut BifContext<'_>,
    table: &Term,
    spec: &Term,
    limit: Option<&Term>,
    reverse: bool,
) -> Result<BifResult, BeamError> {
    select_objects(context, table, spec, limit, reverse)
}

/// Run [`select`] over the match specification `spec`.
fn select_objects(
    context: &mut BifContext<'_>,
    table: &Term,
    spec: &Term,
    limit: Option<&Term>,
    reverse: bool,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let chunk = match limit.map(Term::small_integer) {
        None => 0,
        Some(Some(chunk)) if chunk > 0 => chunk as usize,
        Some(_) => return Ok(BifResult::badarg()),
    };
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    // The walk keeps the store lock, so the handle is a clone.
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let Some(id) = ets.id(atoms, table) else {
        return Ok(denied(EtsTables::missing(table)));
    };
    let name = table;
    let table = match ets.read(atoms, &Term::Reference(id), caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            let arguments = select_trap_arguments(id, spec, chunk);
            return Ok(retry(context, lacking, "select_trap", arguments));
        }
    };
    let Some(compiled) = compile_spec(atoms, spec)? else {
        return Ok(BifResult::badarg());
    };
    if table.table_type() == TableType::OrderedSet {
        let chunk = chunk as i64;
        let turn = tree::first_turn(atoms, &table, &compiled, chunk, reverse)?;
        drop(table);
        drop(ets);
        return tree::finish(context, (name, id), &compiled, chunk, reverse, turn);
    }
    // A key position below 1 compares as a large unsigned number with the
    // arity of the pattern (`db_getkey`, `erl_db_util.c:3940-3960`).
    let keypos = usize::try_from(table.keypos())
        .ok()
        .filter(|&keypos| keypos > 0)
        .unwrap_or(usize::MAX);
    let nothing = if compiled.matches_nothing(keypos) {
        true
    } else if let Some(key) = compiled.bound_key(keypos) {
        match table.key_bucket(atoms, key) {
            Some(bucket) => {
                let mut state = MatchState::new(chunk, table.writes());
                let mut iterations = MATCH_ITERATIONS;
                state.read(atoms, &compiled, table.chain(bucket), &mut iterations);
                drop(table);
                drop(ets);
                return match_end(context, id, &compiled, state, -1, iterations);
            }
            None => true,
        }
    } else {
        false
    };
    if nothing {
        return Ok(Value(if chunk > 0 {
            Term::static_atom("$end_of_table")
        } else {
            Term::nil()
        }));
    }
    let state = MatchState::new(chunk, table.writes());
    let mut bucket = 0;
    let mut state = state;
    let mut iterations = MATCH_ITERATIONS;
    let end = match_walk(
        atoms,
        &table,
        &compiled,
        &mut bucket,
        &mut state,
        &mut iterations,
    );
    drop(table);
    drop(ets);
    match_finish(
        context,
        id,
        spec,
        &compiled,
        state,
        end.ok_or(bucket),
        iterations,
    )
}

/// Return the arguments of the trap of a select that found a busy table,
/// as `handle_lacking_permission` makes the call run again in a later turn
/// (`erl_db.c:769-798`). The change count `u64::MAX` marks the restart: the
/// insert changes the table, and the resumed call reads the current count
/// ([`match_trap`]).
fn select_trap_arguments(id: Reference, spec: &Term, chunk: usize) -> Vec<Term> {
    vec![
        Term::Reference(id),
        spec.clone(),
        Term::Integer(0),
        Term::Integer(chunk as i64),
        Term::nil(),
        Term::Integer(0),
        Term::Integer(u64::MAX as i64),
    ]
}

/// Run the trap export `ets:select_trap/7` of a match: the table reference,
/// the match specification, the next bucket, the limit or 0, the matches so
/// far, the number of matches, and the change count of the table. ERTS traps
/// to `ets:select_trap/1` with a tuple that holds a magic reference to the
/// compiled match specification (`erl_db.c:3883-3930`;
/// `erl_db_hash.c:2264-2307`), and PBeam compiles the specification again in
/// each turn. The compile reads only the specification, so its work does not
/// grow with the table.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_trap(&mut context, id, &spec, 300, MatchState::new(0, 1000))?;
/// assert!(matches!(result, BifResult::Value(_) | BifResult::TrapBif { .. }));
/// ```
///
/// # Errors
///
/// See [`match_finish`].
pub(super) fn match_trap(
    context: &mut BifContext<'_>,
    id: Reference,
    spec: &Term,
    bucket: usize,
    state: MatchState,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    // The walk keeps the store lock, so the handle is a clone.
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let table = match ets.read(atoms, &Term::Reference(id), caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            let arguments = vec![
                Term::Reference(id),
                spec.clone(),
                Term::Integer(bucket as i64),
                Term::Integer(state.chunk as i64),
                state.matches.clone(),
                Term::Integer(state.got as i64),
                Term::Integer(state.writes as i64),
            ];
            return Ok(retry(context, lacking, "select_trap", arguments));
        }
    };
    let restart = state.writes == u64::MAX;
    if !restart && table.writes() != state.writes {
        // ponytail: ERTS fixes the table; see "ETS tables" in docs/erts-differences.md.
        return Err(error(
            "unsupported ets:match/2,3 or ets:select/2,3 on a table that changed while the match trapped",
        ));
    }
    let state = MatchState {
        writes: table.writes(),
        ..state
    };
    let Some(compiled) = compile_spec(atoms, spec)? else {
        return Ok(BifResult::badarg());
    };
    let mut bucket = bucket;
    let mut state = state;
    let mut iterations = MATCH_ITERATIONS;
    let end = match_walk(
        atoms,
        &table,
        &compiled,
        &mut bucket,
        &mut state,
        &mut iterations,
    );
    drop(table);
    drop(ets);
    match_finish(
        context,
        id,
        spec,
        &compiled,
        state,
        end.ok_or(bucket),
        iterations,
    )
}

/// Run `ets:select(Continuation)`, as `ets_select_1` and
/// `db_select_continue_hash` (`erl_db.c:3931-3998`;
/// `erl_db_hash.c:2411-2478`) do. The continuation
/// `{Table, Next, Limit, Program, Rest, RestSize}` continues the match that
/// made it. `'$end_of_table'` gives `'$end_of_table'`. Each other term that
/// is not a 6-tuple with a small next bucket, a small limit, a program
/// reference of a live table, a list, and a small size raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = select_continue(&mut context, &Term::static_atom("$end_of_table"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("$end_of_table")));
/// ```
///
/// # Errors
///
/// Return the errors of [`match_finish`].
pub(super) fn select_continue(
    context: &mut BifContext<'_>,
    continuation: &Term,
    bif: &'static str,
) -> Result<BifResult, BeamError> {
    if matches!(continuation, Term::Atom(atom) if atom.is_static("$end_of_table")) {
        return Ok(Value(Term::static_atom("$end_of_table")));
    }
    let Term::Tuple(elements) = continuation else {
        return Ok(BifResult::badarg());
    };
    let Some(table) = elements.first() else {
        return Ok(BifResult::badarg());
    };
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    // ERTS takes the table with `DB_GET_TABLE`, which helps a clear that
    // does not finish and traps the continuation
    // (`erl_db.c:89-114`, `769-798`, `3965`).
    let atoms = &context.scheduler.atoms;
    // The walk keeps the store lock, so the handle is a clone.
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let Some(id) = ets.id(atoms, table) else {
        return Ok(denied(EtsTables::missing(table)));
    };
    let found = match ets.read(atoms, &Term::Reference(id), caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            return Ok(retry(context, lacking, bif, vec![continuation.clone()]));
        }
    };
    if found.table_type() == TableType::OrderedSet {
        let Some(state) = tree::Continuation::read(elements) else {
            return Ok(BifResult::badarg());
        };
        let Some(compiled) = ets.program(state.program) else {
            return Ok(BifResult::badarg());
        };
        let compiled = compiled.clone();
        let (chunk, reverse) = (state.chunk, state.reverse);
        let turn = tree::next_turn(atoms, &found, &compiled, state)?;
        drop(found);
        drop(ets);
        return tree::finish(context, (table, id), &compiled, chunk, reverse, turn);
    }
    let [_, next, chunk, program, rest, size] = elements.as_ref() else {
        return Ok(BifResult::badarg());
    };
    let (Term::Integer(next), Term::Integer(chunk), Term::Reference(program), Term::Integer(size)) =
        (next, chunk, program, size)
    else {
        return Ok(BifResult::badarg());
    };
    if *chunk < 0 || *size < 0 || !(rest.is_nil() || rest.cons_cell().is_some()) {
        return Ok(BifResult::badarg());
    }
    let writes = found.writes();
    let bucket = found.bucket_count();
    let Some(compiled) = ets.program(*program) else {
        return Ok(BifResult::badarg());
    };
    let spec = match_specification_of(compiled);
    let compiled = compiled.clone();
    if *next >= 0 && *next as usize >= bucket {
        return Ok(BifResult::badarg());
    }
    let state = MatchState {
        chunk: *chunk as usize,
        matches: rest.clone(),
        got: *size as usize,
        writes,
        resumed: true,
    };
    if *next < 0 || (state.chunk > 0 && state.got >= state.chunk) {
        drop(found);
        drop(ets);
        return match_end(context, id, &compiled, state, *next, MATCH_ITERATIONS);
    }
    let table = found;
    let mut bucket = *next as usize;
    let mut state = state;
    let mut iterations = MATCH_ITERATIONS;
    let end = match_walk(
        atoms,
        &table,
        &compiled,
        &mut bucket,
        &mut state,
        &mut iterations,
    );
    drop(table);
    drop(ets);
    match_finish(
        context,
        id,
        &spec,
        &compiled,
        state,
        end.ok_or(bucket),
        iterations,
    )
}

/// Run the trap export `ets:select_delete_trap/1` of a select delete on an
/// `ordered_set` table with the continuation
/// `{Table, LastKey, LeastKey, Program, Count}`, as
/// `ets_select_delete_trap_1` does (`erl_db.c:3387-3427`): a table that the
/// caller cannot write any more raises `badarg`.
///
/// # Errors
///
/// Return the errors of [`tree::select_delete_next`].
pub(super) fn select_delete_trap(
    context: &mut BifContext<'_>,
    arguments: &[Term],
    (table, last, least, program, count): (&Term, &Term, &Term, Reference, i64),
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let mut taken = match ets.write(atoms, table, caller, reds) {
        Ok(taken) => taken,
        Err(lacking) => {
            let arguments = arguments.to_vec();
            return Ok(retry(context, lacking, "select_delete_trap", arguments));
        }
    };
    let id = ets.id(atoms, table).expect("the table exists");
    let compiled = ets
        .program(program)
        .cloned()
        .expect("a trapped select delete keeps its program");
    let (last, least) = (last.clone(), least.clone());
    let turn = tree::select_delete_next(atoms, &mut taken, &compiled, last, least, count)?;
    drop(taken);
    drop(ets);
    tree::delete_finish(context, (table, id), &compiled, Some(program), turn)
}

/// Run the trap export `ets:reverse/3` of a select with a limit on an
/// `ordered_set` table ([`tree::reverse`]).
pub(super) fn reverse(
    context: &mut BifContext<'_>,
    list: &Term,
    result: &Term,
    rest: &Term,
) -> BifResult {
    tree::reverse(context, list, result, rest)
}

/// Return the specification term of a compiled match specification. PBeam
/// keeps the term of the compiled program, so a continuation resumes with
/// the same specification.
fn match_specification_of(compiled: &MatchSpec) -> Term {
    compiled.specification()
}

/// The state of a match across its turns.
pub(super) struct MatchState {
    /// The limit of `ets:match/3` or `ets:select/3`, or 0 for a call
    /// without a limit.
    pub(super) chunk: usize,
    /// The list of the matches so far, the last match first, as
    /// `select_chunk_on_match_res` (`erl_db_hash.c:2174-2185`) puts each
    /// match before the earlier ones.
    pub(super) matches: Term,
    /// The number of matches so far.
    pub(super) got: usize,
    /// The change count of the table in the first turn.
    pub(super) writes: u64,
    /// Whether a trap came before this turn. Then the rest of `ets:match/3`
    /// is built from the front of the matches, so it is in reverse order
    /// (`erl_db_hash.c:2380-2388`).
    pub(super) resumed: bool,
}

impl MatchState {
    /// Return the state of a match with no match yet.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(MatchState::new(0, 0).got, 0);
    /// ```
    fn new(chunk: usize, writes: u64) -> MatchState {
        MatchState {
            chunk,
            matches: Term::nil(),
            got: 0,
            writes,
            resumed: false,
        }
    }

    /// Match each object of `objects` with `spec` and use one iteration
    /// for each object (`erl_db_hash.c:1900-1916`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// state.read(&atoms, &spec, table.chain(0), &mut iterations);
    /// ```
    fn read<'a>(
        &mut self,
        atoms: &AtomTable,
        spec: &MatchSpec,
        objects: impl Iterator<Item = &'a [Term]>,
        iterations: &mut i64,
    ) {
        for object in objects {
            if let Some(found) = spec.run(atoms, object) {
                self.matches = Term::cons(found, std::mem::replace(&mut self.matches, Term::nil()));
                self.got += 1;
            }
            *iterations -= 1;
        }
    }
}

/// Read the buckets of the table `id` from `bucket` up, as `match_traverse`
/// and `match_traverse_continue` (`erl_db_hash.c:1835-2071`) do. An empty
/// bucket uses no iteration. After each bucket the turn ends when it was
/// the last bucket, or when the matches reach the limit of
/// `ets:match/3` or `ets:select/3`. Else, when the turn used its 1000
/// iterations, the call uses all reductions and traps to
/// `ets:select_trap/7` with the next bucket ([`match_trap`]). A bucket is
/// never split across turns, so a trap keeps the position and no turn reads
/// a bucket again.
///
/// # Examples
///
/// ```rust,ignore
/// let mut bucket = 0;
/// let end = match_walk(&atoms, &table, &compiled, &mut bucket, &mut state, &mut iterations);
/// assert_eq!(end, Some(-1));
/// ```
///
/// The caller holds the read lock of the table from its check of the change
/// count and the bucket through the walk, so no other scheduler clears or
/// deletes the table between them.
fn match_walk(
    atoms: &AtomTable,
    table: &Table,
    compiled: &MatchSpec,
    bucket: &mut usize,
    state: &mut MatchState,
    iterations: &mut i64,
) -> Option<i64> {
    loop {
        state.read(atoms, compiled, table.chain(*bucket), iterations);
        *bucket += 1;
        if *bucket == table.bucket_count() {
            break Some(-1);
        }
        if state.chunk > 0 && state.got >= state.chunk {
            break Some(*bucket as i64);
        }
        if *iterations <= 0 {
            break None;
        }
    }
}

/// End the turn of a match after [`match_walk`] and the release of the
/// locks: `Ok(next)` gives the next bucket of the end, and `Err(bucket)` a
/// trap that continues at `bucket`.
///
/// # Errors
///
/// Return the errors of [`match_end`].
fn match_finish(
    context: &mut BifContext<'_>,
    id: Reference,
    spec: &Term,
    compiled: &MatchSpec,
    state: MatchState,
    end: Result<i64, usize>,
    iterations: i64,
) -> Result<BifResult, BeamError> {
    match end {
        Ok(next) => match_end(context, id, compiled, state, next, iterations),
        Err(bucket) => {
            context
                .scheduler
                .current_process_mut()
                .bump_all_reductions();
            Ok(BifResult::TrapBif {
                module: "ets",
                function: "select_trap",
                arguments: vec![
                    Term::Reference(id),
                    spec.clone(),
                    Term::Integer(bucket as i64),
                    Term::Integer(state.chunk as i64),
                    state.matches,
                    Term::Integer(state.got as i64),
                    Term::Integer(state.writes as i64),
                ],
            })
        }
    }
}

/// End a match whose next bucket is `next`, or -1 after the last bucket,
/// with `iterations` left, as `select_chunk_on_loop_ended` and
/// `select_chunk_continue_on_loop_ended` (`erl_db_hash.c:2187-2262`,
/// `2364-2409`) do. The call uses one reduction for each iteration. Then:
///
/// - A call without a limit returns the matches.
/// - A call with a limit keeps the first matches of the list over the limit
///   as the rest. It returns the other matches and the continuation
///   `{Table, Next, Limit, Program, Rest, RestSize}` when there is a rest
///   or a next bucket, else `{Matches, '$end_of_table'}`, or
///   `'$end_of_table'` with no match. ERTS puts a magic reference to the
///   compiled specification in `Program`, and PBeam puts a new reference
///   there and keeps the compiled specification under it.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_end(&mut context, id, &compiled, MatchState::new(1, 0), -1, 1000)?;
/// assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("$end_of_table")));
/// ```
///
/// # Errors
///
/// Return a runtime error when the references of the run are used up.
fn match_end(
    context: &mut BifContext<'_>,
    id: Reference,
    compiled: &MatchSpec,
    state: MatchState,
    next: i64,
    iterations: i64,
) -> Result<BifResult, BeamError> {
    let reductions = i32::try_from(MATCH_ITERATIONS - iterations).unwrap_or(i32::MAX);
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(reductions);
    if state.chunk == 0 {
        return Ok(Value(state.matches));
    }
    let mut matches = state.matches;
    let mut rest = Vec::new();
    for _ in state.chunk..state.got {
        let (head, tail) = matches.cons_cell().expect("the list holds each match");
        rest.push(head.clone());
        matches = tail.clone();
    }
    if state.resumed {
        rest.reverse();
    }
    if !rest.is_empty() || next >= 0 {
        let program = context.scheduler.make_ref()?;
        let size = rest.len() as i64;
        (context.scheduler.ets.exclusive()).remember_program(id, program, compiled.clone());
        let continuation = Term::tuple(vec![
            Term::Reference(id),
            Term::Integer(next),
            Term::Integer(state.chunk as i64),
            Term::Reference(program),
            Term::list_from(rest),
            Term::Integer(size),
        ]);
        return Ok(Value(Term::tuple(vec![matches, continuation])));
    }
    let end = Term::static_atom("$end_of_table");
    Ok(Value(if matches.is_nil() {
        end
    } else {
        Term::tuple(vec![matches, end])
    }))
}

/// Run `ets:next(Table, Key)` or `ets:prev(Table, Key)`, as `ets_next_2`
/// and `ets_prev_2` (`erl_db.c:1216-1234`, `1309-1327`) do. A hash table
/// gives its keys in the chain order of its buckets, and `prev` is the same
/// as `next` (`erl_db_hash.c:854-856`, `1147-1200`). A key that the table
/// does not hold raises `badarg`; after the last key the atom
/// `'$end_of_table'` comes.
///
/// # Examples
///
/// ```rust,ignore
/// let result = next(&mut context, &table, &test_atom("missing"))?;
/// assert!(matches!(result, BifResult::Raise { .. }));
/// ```
/// Run `ets:safe_fixtable/2` as ERTS `ets_safe_fixtable_2` does
/// (`erl_db.c:1136-1164`): `true` fixes the table for the caller, `false`
/// unfixes it, and each of these gives `true`. A second argument that is
/// not `true` or `false`, and a term that is not a table of the run, raise
/// `badarg`. ERTS reads the table before it checks the second argument.
///
/// While a table is fixed, `ets:delete/2` keeps the key of each deleted
/// object as a ghost, so `ets:next/2` from that key continues the
/// traversal, and the table does not split or join a bucket
/// (`erl_db_hash.c:49-60`). The exit of a process removes its fixations.
///
/// # Examples
///
/// ```rust,ignore
/// let result = safe_fixtable(&mut context, &table, &Term::static_atom("true"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("true")));
/// ```
///
/// # Errors
///
/// See [`crate::runtime::ets::EtsTables::read`] for a table that a list insert or a clear holds busy.
pub(super) fn safe_fixtable(
    context: &mut BifContext<'_>,
    table: &Term,
    flag: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    // ERTS reads the table before it checks the second argument
    // (`erl_db.c:1146-1161`).
    // The check and the fix take one lock, so no other scheduler deletes
    // the table between them, as ERTS keeps the table lock
    // (`erl_db.c:1146-1162`).
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let mut ets = store.exclusive();
    if let Err(lacking) = ets.read(atoms, table, caller, reds) {
        return Ok(retry(
            context,
            lacking,
            "safe_fixtable",
            vec![table.clone(), flag.clone()],
        ));
    }
    let id = (ets.id(atoms, table)).expect("the caller found the table");
    let Term::Atom(flag) = flag else {
        return Ok(BifResult::badarg());
    };
    if flag.is_one_of(&["true"]) {
        ets.fix_table(id, caller);
    } else if flag.is_one_of(&["false"]) {
        ets.unfix_table(id, caller);
    } else {
        return Ok(BifResult::badarg());
    }
    Ok(Value(Term::static_atom("true")))
}

/// Run one of the eight walk BIFs of a table: `ets:first/1`,
/// `ets:last/1`, `ets:next/2`, `ets:prev/2`, and their `_lookup` forms, as
/// `ets_first_1`, `ets_last_1`, `ets_next_2`, `ets_prev_2`, and the lookup
/// BIFs do (`erl_db.c:1170-1350`). `args` holds the table and, for a step,
/// the key. `backward` selects `last` or `prev`, and `lookup` the form that
/// returns `{Key, Objects}`.
///
/// - A table that does not exist or that the caller cannot read raises
///   `badarg` (`erl_db.c:769-912`).
/// - An `ordered_set` walks its keys in the term order (`db_first_tree`,
///   `db_next_tree`, `db_last_tree`, `db_prev_tree`,
///   `erl_db_tree.c:635-757`). A step from a key that is not in the
///   table gives the next key in the order.
/// - A hash table walks its buckets (`db_first_hash_common`,
///   `erl_db_hash.c:1117-1140`, `db_next_hash`). `last` acts as `first`
///   and `prev` as `next` (`erl_db_hash.c:853-856`). A step from a key that is not in the table
///   raises `badarg`.
/// - The end of the table gives `'$end_of_table'`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = walk(&mut context, "last", vec![table], true, false)?;
/// assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("$end_of_table")));
/// ```
pub(super) fn walk(
    context: &mut BifContext<'_>,
    name: &'static str,
    args: Vec<Term>,
    backward: bool,
    lookup: bool,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let table = match ets.read(atoms, &args[0], caller, reds) {
        Ok(table) => table,
        Err(lacking) => return Ok(retry(context, lacking, name, args)),
    };
    let found = match (table.table_type(), args.get(1)) {
        (TableType::OrderedSet, None) => table.ordered_end(!backward)?,
        // A tree table rejects the end marker as a key
        // (`erl_db_tree.c:654-655`, `729-730`).
        (TableType::OrderedSet, Some(key)) if *key == Term::static_atom("$end_of_table") => {
            return Ok(BifResult::badarg());
        }
        (TableType::OrderedSet, Some(key)) => table.ordered_step(atoms, key, !backward)?,
        (_, None) => table.first_key(),
        (_, Some(key)) => match table.next_key(atoms, key) {
            Some(next) => next,
            None => return Ok(BifResult::badarg()),
        },
    };
    let Some(key) = found else {
        return Ok(Value(Term::static_atom("$end_of_table")));
    };
    if !lookup {
        return Ok(Value(key));
    }
    let objects = table
        .objects(atoms, &key)
        .into_iter()
        .map(|object| Term::tuple(object.to_vec()));
    Ok(Value(Term::tuple(vec![key, Term::list_from(objects)])))
}

/// Run `ets:update_element/3` or `ets:update_element/4`, as
/// `ets_update_element_3` and `ets_update_element_4` (`erl_db.c:1464-1491`)
/// do with `do_update_element` (`erl_db.c:1371-1458`):
///
/// - A table that does not exist or that the caller cannot write raises
///   `badarg` (`erl_db.c:769-912`).
/// - A default object that is not a tuple with at least `keypos` elements
///   raises `badarg` (`erl_db.c:1483-1488`).
/// - The table type must be `set` (`erl_db.c:1381-1385`).
/// - Each update is a `{Pos, Value}` tuple with a small `Pos`. `Pos` equal
///   to the key position, below 1, or above the object arity raises
///   `badarg` (`erl_db.c:1399-1428`).
/// - A missing key gives `false`, or, with a default object, inserts the
///   default object with the requested key (`erl_db.c:1393-1397`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = update_element(&mut context, &table, &key, &Term::tuple(vec![Term::Integer(2), Term::Integer(9)]), None)?;
/// assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("true")));
/// ```
pub(super) fn update_element(
    context: &mut BifContext<'_>,
    table: &Term,
    key: &Term,
    spec: &Term,
    default: Option<&Term>,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let mut table = match ets.write(atoms, table, caller, reds) {
        Ok(table) => table,
        Err(lacking) => {
            let mut arguments = vec![table.clone(), key.clone(), spec.clone()];
            if let Some(default) = default {
                arguments.push(default.clone());
            }
            return Ok(retry(context, lacking, "update_element", arguments));
        }
    };
    let keypos = table.keypos();
    // ERTS takes the table before it checks the default, and compares the
    // unsigned arity with the C `int` key position, so a negative key
    // position rejects each default (`erl_db.c:1481-1488`).
    let default_elements = match default {
        Some(Term::Tuple(elements)) if elements.len() as u32 >= keypos as u32 => Some(elements),
        Some(_) => return Ok(denied("default")),
        None => None,
    };
    // A table of another type sets the cause `table_type`, but the
    // `DB_ERROR_BADPARAM` return raises `badarg` with no cause
    // (`erl_db.c:1381-1385`, `1454-1457`).
    if !matches!(table.table_type(), TableType::Set | TableType::OrderedSet) {
        return Ok(BifResult::badarg());
    }
    // `do_update_element` finds the key before it checks the update list, so
    // a missing key without a default gives `false` (`erl_db.c:1393-1397`).
    // A default inserts no object when an update is bad
    // (`erl_db_hash.c:3972`).
    let existing = table.object_arity(atoms, key);
    let arity = match existing {
        Some(arity) => arity,
        None => match default_elements {
            None => return Ok(Value(Term::static_atom("false"))),
            Some(elements) => elements.len(),
        },
    };
    let updates = match update_specs(spec, keypos, arity) {
        Ok(updates) => updates,
        Err(Some(cause)) => return Ok(denied(cause)),
        Err(None) => return Ok(BifResult::badarg()),
    };
    if existing.is_none() {
        let elements = default_elements.expect("the missing key has a default");
        let mut object = elements.as_ref().to_vec();
        let Some(index) = usize::try_from(keypos)
            .ok()
            .and_then(|keypos| keypos.checked_sub(1))
        else {
            return Ok(BifResult::badarg());
        };
        object[index] = key.clone();
        table.insert(atoms, object);
    }
    table.update_elements(atoms, key, &updates);
    Ok(Value(Term::static_atom("true")))
}

/// Return the positions and values of an `update_element` specification
/// for an object of arity `arity`, as the loop of `do_update_element`
/// (`erl_db.c:1401-1428`) checks it, one element after another. An element
/// that is not a `{Pos, Value}` with a small position, or an improper tail,
/// gives `Err(None)`: `badarg` with no cause. A position equal to the key
/// position gives the cause `keypos`, and a position out of range gives the
/// cause `position`.
fn update_specs(
    spec: &Term,
    keypos: i32,
    arity: usize,
) -> Result<Vec<(usize, Term)>, Option<&'static str>> {
    let single;
    let mut list = if matches!(spec, Term::Tuple(_)) {
        single = Term::list_from(vec![spec.clone()]);
        &single
    } else {
        spec
    };
    let mut updates = Vec::new();
    while !list.is_nil() {
        let (item, tail) = list.cons_cell().ok_or(None)?;
        let Term::Tuple(elements) = item else {
            return Err(None);
        };
        let [position, value] = elements.as_ref() else {
            return Err(None);
        };
        let position = position.small_integer().ok_or(None)?;
        if position == i64::from(keypos) {
            return Err(Some("keypos"));
        }
        let position = (usize::try_from(position).ok())
            .filter(|position| (1..=arity).contains(position))
            .ok_or(Some("position"))?;
        updates.push((position, value.clone()));
        list = tail;
    }
    Ok(updates)
}

/// Run `ets:info(Table)`, as `ets_info_1` (`erl_db.c:4399-4517`) does:
/// return the list of `{Item, Value}` pairs for the table, in the order of
/// the `fields` array, reversed by the cons build (`erl_db.c:4488-4516`).
/// A free name gives `undefined`; a term that is not an atom or a reference
/// raises `badarg`. The items `binary`, `data`, and `stats` raise a runtime
/// error: PBeam does not run them yet.
///
/// # Examples
///
/// ```rust,ignore
/// let result = info_all(&mut context, &table)?;
/// assert!(matches!(result, BifResult::Value(value) if value.is_nil() || value.cons_cell().is_some()));
/// ```
///
/// # Errors
///
/// Return a runtime error for the items `binary`, `data`, and `stats`.
pub(super) fn info_all(context: &mut BifContext<'_>, table: &Term) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let found = match ets.info(atoms, table, caller, reds) {
        Ok(found) => found,
        Err(lacking @ Lacking::Busy { .. }) => {
            return Ok(retry(context, lacking, "info", vec![table.clone()]));
        }
        // A table that no longer exists gives `undefined`; another term
        // raises with the cause `type` (`erl_db.c:4434-4441`, `4544-4551`).
        Err(Lacking::Denied("id")) => return Ok(Value(Term::static_atom("undefined"))),
        Err(Lacking::Denied(cause)) => return Ok(denied(cause)),
    };
    let id = (ets.id(atoms, table)).expect("the caller found the table");
    let named = ets.named(id);
    let items = [
        ("id", Term::Reference(id)),
        (
            "decentralized_counters",
            boolean_term(found.decentralized_counters()),
        ),
        ("read_concurrency", boolean_term(found.read_concurrency())),
        ("write_concurrency", found.write_concurrency().clone()),
        ("compressed", boolean_term(found.compressed())),
        ("memory", Term::Integer(found.memory_words() as i64)),
        ("owner", Term::Pid(found.owner())),
        (
            "heir",
            found.heir().map_or(Term::static_atom("none"), Term::Pid),
        ),
        (
            "name",
            Term::Atom(
                atoms
                    .get(found.name())
                    .expect("the table name atom is in the table"),
            ),
        ),
        ("size", Term::from_bigint(BigInt::from(found.size()))),
        ("node", Term::static_atom("nonode@nohost")),
        ("named_table", boolean_term(named)),
        ("type", Term::static_atom(found.table_type().name())),
        ("keypos", Term::Integer(i64::from(found.keypos()))),
        ("protection", Term::static_atom(access_name(found.access()))),
    ];
    let pairs: Vec<Term> = items
        .into_iter()
        .map(|(item, value)| Term::tuple(vec![Term::static_atom(item), value]))
        .collect();
    Ok(Value(Term::list_from(pairs)))
}

/// Return the atom `true` or `false` for `value`.
fn boolean_term(value: bool) -> Term {
    Term::static_atom(if value { "true" } else { "false" })
}

/// Return the atom of the access mode, as `ets:info/2` gives it.
fn access_name(access: Access) -> &'static str {
    match access {
        Access::Public => "public",
        Access::Protected => "protected",
        Access::Private => "private",
    }
}

/// Run `ets:internal_delete_all/2`, the BIF of `ets:delete_all_objects/1`
/// (`lib/stdlib/src/ets.erl:386-396`), as `ets_internal_delete_all_2`
/// (`erl_db.c:3234-3299`) does: a table that the caller cannot write raises
/// `badarg`; the argument `undefined` removes each object and returns their
/// count, and each other argument is returned, as the trap argument
/// `NItemsHolder` gives (`erl_db.c:3284-3297`). The call uses the ERTS free
/// cost: 32 reductions for each segment and 1 for each object
/// (`erl_db_hash.c:3122-3149`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = internal_delete_all(&mut context, &table, &Term::static_atom("undefined"))?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(_))));
/// ```
pub(super) fn internal_delete_all(
    context: &mut BifContext<'_>,
    table: &Term,
    holder: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    // The signed budget of `ets_internal_delete_all_2` (`erl_db.c:3236`).
    let left = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let Some(id) = ets.id(atoms, table) else {
        return Ok(denied(EtsTables::missing(table)));
    };
    let mut found = match ets.write(atoms, table, caller, left) {
        Ok(table) => table,
        // The call keeps its own arguments: a call that has not started its
        // clear yet runs it after the clear of the other operation
        // (`erl_db.c:769-798`), and a call after its own trap returns the
        // count that it holds.
        Err(lacking) => {
            let arguments = vec![Term::Reference(id), holder.clone()];
            return Ok(retry(context, lacking, "internal_delete_all", arguments));
        }
    };
    if !matches!(holder, Term::Atom(atom) if atom.is_static("undefined")) {
        return Ok(Value(holder.clone()));
    }
    let count = found.size();
    let reductions = found.delete_all_reductions();
    if found.is_fixed() && found.table_type() != TableType::OrderedSet {
        let units = usize::try_from(count).unwrap_or(usize::MAX);
        let done = found.step_clear(atoms, caller, units, left).0;
        drop(found);
        drop(ets);
        if !done {
            // The clear marks 8 buckets for each reduction. A clear that
            // does not finish marks the table busy, uses the turn, and
            // traps to itself (`erl_db.c:3255-3279`).
            context
                .scheduler
                .current_process_mut()
                .bump_all_reductions();
            return Ok(BifResult::TrapBif {
                module: "ets",
                function: "internal_delete_all",
                arguments: vec![Term::Reference(id), Term::from_bigint(BigInt::from(count))],
            });
        }
        context
            .scheduler
            .current_process_mut()
            .bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
        return Ok(Value(Term::from_bigint(BigInt::from(count))));
    }
    found.clear(&context.scheduler.atoms);
    drop(found);
    drop(ets);
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(i32::try_from(reductions).unwrap_or(i32::MAX));
    Ok(Value(Term::from_bigint(BigInt::from(count))))
}

/// Run `ets:internal_select_delete/2`, the BIF of `ets:select_delete/2`
/// (`lib/stdlib/src/ets.erl:1849-1861`), as `ets_internal_select_delete_2`
/// (`erl_db.c:3433-3475`) does: a table that the caller cannot write raises
/// `badarg`; a match specification that does not compile raises `badarg`;
/// each object whose result is `true` is removed, and the count of removed
/// objects comes back. The traversal uses one iteration for each object and
/// charges `1000 - iterations_left` reductions (`erl_db_hash.c:2677-2709`).
///
/// # Examples
///
/// ```rust,ignore
/// let result = internal_select_delete(&mut context, &table, &spec)?;
/// assert!(matches!(result, BifResult::Value(Term::Integer(_))));
/// ```
///
/// # Errors
///
/// Return a runtime error for a guard function that PBeam does not run yet.
pub(super) fn internal_select_delete(
    context: &mut BifContext<'_>,
    table: &Term,
    spec: &Term,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let reds = context
        .scheduler
        .current_process_mut()
        .bif_reductions_signed();
    let atoms = &context.scheduler.atoms;
    let store = Arc::clone(&context.scheduler.ets);
    let ets = store.shared();
    let Some(found) = ets.find(atoms, table) else {
        return Ok(denied(EtsTables::missing(table)));
    };
    drop(found);
    let mut taken = match ets.write(atoms, table, caller, reds) {
        Ok(taken) => taken,
        Err(lacking) => {
            let arguments = vec![table.clone(), spec.clone()];
            return Ok(retry(context, lacking, "internal_select_delete", arguments));
        }
    };
    // The write lock covers the whole call, as in ERTS
    // (`erl_db.c:3443-3458`), so another scheduler cannot clear the table
    // and shrink its buckets before the walk.
    let keypos = taken.keypos();
    let buckets = taken.bucket_count();
    let Some(compiled) = compile_spec(atoms, spec)? else {
        return Ok(BifResult::badarg());
    };
    let keypos = usize::try_from(keypos)
        .ok()
        .filter(|&keypos| keypos > 0)
        .unwrap_or(usize::MAX);
    if taken.table_type() == TableType::OrderedSet {
        let id = ets.id(atoms, table).expect("the table exists");
        let turn = tree::select_delete(atoms, &mut taken, &compiled)?;
        drop(taken);
        drop(ets);
        return tree::delete_finish(context, (table, id), &compiled, None, turn);
    }
    if compiled.matches_nothing(keypos) {
        return Ok(Value(Term::Integer(0)));
    }
    let table = &mut *taken;
    let mut iterations = 0_usize;
    let mut matches = Vec::new();
    for bucket in 0..buckets {
        for object in table.chain(bucket) {
            if compiled.run(atoms, object) == Some(Term::static_atom("true")) {
                matches.push(object.to_vec());
            }
            iterations += 1;
        }
    }
    let mut removed = 0;
    table.begin_select_delete();
    for object in &matches {
        if table.delete_object(atoms, object) {
            removed += 1;
        }
    }
    table.shrink();
    drop(taken);
    drop(ets);
    context
        .scheduler
        .current_process_mut()
        .bump_reductions(i32::try_from(iterations).unwrap_or(i32::MAX));
    Ok(Value(Term::Integer(removed)))
}
