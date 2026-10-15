//! Bodies of the `ets` BIFs that do not fit on one table line.

use super::BifResult::Value;
use super::*;
use crate::runtime::ets::pattern::Pattern;
use crate::runtime::ets::{Access, Operation, Table, TableType};
use crate::term::Reference;

/// The fewest and the most locks that the hidden option
/// `{write_concurrency, {debug_hash_fixed_number_of_locks, N}}` accepts
/// (`DB_WRITE_CONCURRENCY_MIN_LOCKS` and `DB_WRITE_CONCURRENCY_MAX_LOCKS`,
/// `erl_db.c:65-66`).
const LOCKS: std::ops::RangeInclusive<i64> = 1..=32768;

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
    let mut table_type = Some(TableType::Set);
    let mut named = false;
    let mut access = Access::Protected;
    let mut keypos = 1;
    let mut explicit_locks = false;
    let mut heir = None;
    for option in options {
        match option {
            Term::Atom(atom) => {
                if atom.is_static("bag") {
                    table_type = Some(TableType::Bag);
                } else if atom.is_static("duplicate_bag") {
                    table_type = Some(TableType::DuplicateBag);
                } else if atom.is_static("ordered_set") {
                    table_type = None;
                } else if atom.is_static("named_table") {
                    named = true;
                } else if atom.is_static("public") {
                    access = Access::Public;
                } else if atom.is_static("private") {
                    access = Access::Private;
                } else if atom.is_one_of(&["set", "protected", "compressed"]) {
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
                    }
                    ("write_concurrency", Term::Tuple(locks)) if is_lock_count(locks) => {
                        explicit_locks = true;
                    }
                    ("read_concurrency" | "decentralized_counters", Term::Atom(value))
                        if value.is_one_of(&["true", "false"]) => {}
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
    if (table_type.is_none() && explicit_locks)
        || (named
            && context
                .scheduler
                .ets
                .name_in_use(context.scheduler.atoms.name_of(name)))
    {
        return Ok(BifResult::badarg());
    }
    let Some(table_type) = table_type else {
        // ponytail: no ordered_set tables; see "ETS tables" in docs/erts-differences.md.
        return Err(error("unsupported ets:new/2 table type ordered_set"));
    };
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
        name.map(|name| context.scheduler.atoms.name_of(name)),
        Table::new(owner, access, keypos, table_type),
    );
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
        let Some((id, found)) = context.scheduler.ets.find(&context.scheduler.atoms, table) else {
            return Ok(BifResult::badarg());
        };
        let (keypos, bag) = (found.keypos(), found.table_type() == TableType::Bag);
        let check = Check::Rest(objects.clone());
        return insert_list(context, id, objects, 0, keypos, bag.then(Term::nil), check);
    }
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) =
        context
            .scheduler
            .ets
            .table(&context.scheduler.atoms, table, caller, Operation::Write)
    else {
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
    let compared = put_objects(&context.scheduler.atoms, table, &tuples, keypos)?;
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
/// let objects = Term::list_from(vec![Term::tuple(vec![test_atom("k")]); 8]);
/// // 8 units of the check and 16 units of the insert: 3 reductions.
/// let check = Check::Rest(objects.clone());
/// assert!(matches!(insert_list(&mut context, id, &objects, 0, 1, None, check)?, BifResult::Value(_)));
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
    mut bag: Option<Term>,
    mut check: Check,
) -> Result<BifResult, BeamError> {
    /// `ITERATIONS_PER_RED` (`erl_db.c:2152`).
    const ITERATIONS_PER_RED: i64 = 8;
    let table = Term::Reference(id);
    let inserting = |bag: &Option<Term>| bag.as_ref().is_some_and(|steps| !steps.is_nil());
    // The number of objects is known only after the check. Until then,
    // `done` takes its place, so that each test of a later phase is false.
    let mut count = match check {
        Check::Rest(_) => done,
        Check::Done(count) => count,
    };
    let insert_phase = match &bag {
        Some(_) => inserting(&bag),
        None => 2 * count < done && done <= 4 * count,
    };
    if insert_phase
        && context
            .scheduler
            .ets
            .find(&context.scheduler.atoms, &table)
            .is_none()
    {
        return Ok(BifResult::badarg());
    }
    let process = context.scheduler.current_process_mut();
    let caller = process.id;
    let budget = ITERATIONS_PER_RED * process.bif_reductions_left()? as i64;
    let mut units = Units { left: budget };
    // The tuples of the checked list, for the insert. The walk is part of
    // the units of the insert.
    let tuples = || -> Vec<&[Term]> {
        (objects.list_iter().expect("a list insert has a list"))
            .map(|object| valid_object(object, keypos).expect("the check found each object good"))
            .collect()
    };
    // After a failed copy, the units of the removal of the copies follow
    // `4 * count`, and a removal that yields leaves `done` above it.
    let mut removal = match &bag {
        Some(_) => done > 2 * count && !inserting(&bag),
        None => done > 4 * count,
    };
    // `None` when the call yields, else the result of the call.
    let result = 'work: {
        if let Check::Rest(rest) = &mut check {
            match units.check(&mut done, rest, keypos) {
                None => break 'work None,
                Some(false) => break 'work Some(BifResult::badarg()),
                Some(true) => (count, check) = (done, Check::Done(done)),
            }
            if bag.is_none() && count as i64 <= units.left {
                let Some(table) = context.scheduler.ets.table(
                    &context.scheduler.atoms,
                    &table,
                    caller,
                    Operation::Write,
                ) else {
                    break 'work Some(BifResult::badarg());
                };
                put_objects(&context.scheduler.atoms, table, &tuples(), keypos)?;
                units.left -= 2 * count as i64;
                break 'work Some(Value(Term::static_atom("true")));
            }
        }
        if done <= 2 * count && !inserting(&bag) {
            if !units.run(&mut done, 2 * count) {
                break 'work None;
            }
            match context.scheduler.ets.table(
                &context.scheduler.atoms,
                &table,
                caller,
                Operation::Write,
            ) {
                Some(table) => {
                    let compared = put_objects(&context.scheduler.atoms, table, &tuples(), keypos)?;
                    if bag.is_some() {
                        let steps = compared.into_iter().flat_map(|compared| {
                            [Term::Integer(1), Term::Integer(1 + compared as i64)]
                        });
                        bag = Some(Term::list_from(steps));
                    }
                }
                None => (removal, done) = (true, 4 * count),
            }
        }
        if !removal {
            let finished = match &mut bag {
                Some(steps) => units.run_steps(&mut done, steps),
                None => units.run(&mut done, 4 * count),
            };
            if !finished {
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
            bag.unwrap_or_else(|| Term::static_atom("false")),
            check.into_term(),
        ],
    }))
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

    /// Use the steps of the list `steps`, each a number of units, and
    /// return true. When a step makes the counter 0 or less and it is not
    /// the last step, as `YCF_CONSUME_REDS` does (`erl_db.c:1939`), stop
    /// after that step, keep the steps that remain in `steps`, and return
    /// false: the call yields there. `done` counts the units of each step.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let (mut units, mut done) = (Units { left: 3 }, 0);
    /// let mut steps = Term::list_from([Term::Integer(4), Term::Integer(1)]);
    /// assert!(!units.run_steps(&mut done, &mut steps));
    /// assert_eq!((done, units.left), (4, -1));
    /// ```
    fn run_steps(&mut self, done: &mut usize, steps: &mut Term) -> bool {
        loop {
            let (step, rest) = steps.cons_cell().expect("a bag insert has a step");
            let step = step.small_integer().expect("a step is a small integer");
            *done += step as usize;
            self.left -= step;
            let last = rest.is_nil();
            *steps = rest.clone();
            if last {
                return true;
            }
            if self.left <= 0 {
                return false;
            }
        }
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
) -> Result<Vec<usize>, BeamError> {
    if keypos == 0 && !tuples.is_empty() {
        // ponytail: ERTS aborts here; see "ETS tables" in docs/erts-differences.md.
        return Err(error(
            "unsupported ets:insert/2 in a table with key position 0",
        ));
    }
    let objects = tuples.iter().map(|elements| elements.to_vec()).collect();
    Ok(table.insert_all(atoms, objects))
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

/// Run `ets:lookup_element(Table, Key, Index, Default)`, as ERTS
/// `ets_lookup_element_4` (`erl_db.c:2972-3001`) and
/// `db_get_element_hash` (`erl_db_hash.c:1544-1608`) do: return the element
/// at `Index` of the object with the key `Key` in a `set` table, the list of
/// the elements at `Index` of the objects with the key in insertion order in
/// another table, or `Default` when the table has no object with that key.
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
    let Some(table) =
        context
            .scheduler
            .ets
            .table(&context.scheduler.atoms, table, caller, Operation::Read)
    else {
        return Ok(BifResult::badarg());
    };
    let Some(index) = index.small_integer().filter(|index| *index >= 1) else {
        return Ok(BifResult::badarg());
    };
    let objects = table.objects(&context.scheduler.atoms, key);
    if objects.is_empty() {
        return Ok(Value(default.clone()));
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
        (Some(mut elements), TableType::Set) => Value(elements.remove(0)),
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
pub(super) fn lookup(context: &mut BifContext<'_>, table: &Term, key: &Term) -> BifResult {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) =
        context
            .scheduler
            .ets
            .table(&context.scheduler.atoms, table, caller, Operation::Read)
    else {
        return BifResult::badarg();
    };
    let objects = table.objects(&context.scheduler.atoms, key);
    Value(Term::list_from(
        objects
            .into_iter()
            .map(|object| Term::tuple(object.to_vec())),
    ))
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
pub(super) fn member(context: &mut BifContext<'_>, table: &Term, key: &Term) -> BifResult {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) =
        context
            .scheduler
            .ets
            .table(&context.scheduler.atoms, table, caller, Operation::Read)
    else {
        return BifResult::badarg();
    };
    let found = table.object(&context.scheduler.atoms, key).is_some();
    Value(Term::static_atom(if found { "true" } else { "false" }))
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
pub(super) fn delete(context: &mut BifContext<'_>, table: &Term, key: &Term) -> BifResult {
    let caller = context.scheduler.current_process_mut().id;
    let Some(table) =
        context
            .scheduler
            .ets
            .table(&context.scheduler.atoms, table, caller, Operation::Write)
    else {
        return BifResult::badarg();
    };
    table.delete(&context.scheduler.atoms, key);
    Value(Term::static_atom("true"))
}

/// Run `ets:info(Table, Item)`, as ERTS `ets_info_2` (`erl_db.c:4523-4579`)
/// does, for the item `type`: return `set`, `bag`, or `duplicate_bag`
/// (`table_info`, `erl_db.c:5457-5469`). The item `binary` traps before the
/// table lookup (`erl_db.c:4541-4542`). Then the lookup asks only for
/// `DB_INFO` (`erl_db_util.h:451`), which each process has. A free name
/// gives `undefined` (`erl_db.c:4544-4547`), and a term that is not an atom
/// or a reference raises `badarg`. After the lookup, an item that
/// `table_info` (`erl_db.c:5447-5625`) does not know raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// let result = info(&mut context, &table, &test_atom("type"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("set")));
/// let result = info(&mut context, &test_atom("missing"), &test_atom("type"))?;
/// assert!(matches!(result, BifResult::Value(value) if value == test_atom("undefined")));
/// assert!(info(&mut context, &table, &test_atom("size")).is_err());
/// ```
///
/// # Errors
///
/// Return a runtime error for the item `binary` and for each other item
/// that ERTS knows. Also return a runtime error for a reference that is not
/// a live table: ERTS gives `undefined` for the reference of a deleted
/// table and raises `badarg` for another reference, and PBeam keeps no
/// record of a deleted table.
pub(super) fn info(
    context: &mut BifContext<'_>,
    table: &Term,
    item: &Term,
) -> Result<BifResult, BeamError> {
    /// The items of `table_info` other than `type` (`erl_db.c:5454-5622`).
    const ITEMS: [&str; 19] = [
        "size",
        "memory",
        "owner",
        "heir",
        "protection",
        "write_concurrency",
        "read_concurrency",
        "name",
        "keypos",
        "node",
        "named_table",
        "compressed",
        "id",
        "decentralized_counters",
        "data",
        "fixed",
        "safe_fixed",
        "safe_fixed_monotonic_time",
        "stats",
    ];
    let is_item = |name| matches!(item, Term::Atom(atom) if atom.is_static(name));
    if is_item("binary") {
        // ponytail: info/2 gives only the type; see "ETS tables" in docs/erts-differences.md.
        return Err(error("unsupported ets:info/2 item binary"));
    }
    let atoms = &context.scheduler.atoms;
    let Some((_, found)) = context.scheduler.ets.find(atoms, table) else {
        return match table {
            Term::Atom(_) => Ok(Value(Term::static_atom("undefined"))),
            // ponytail: no record of a deleted table; see "ETS tables" in docs/erts-differences.md.
            Term::Reference(_) => Err(error(
                "unsupported ets:info/2 of a reference that is not a live table",
            )),
            _ => Ok(BifResult::badarg()),
        };
    };
    if is_item("type") {
        return Ok(Value(Term::static_atom(found.table_type().name())));
    }
    match item {
        Term::Atom(atom) if ITEMS.contains(&atoms.name_of(atom)) => {
            // ponytail: info/2 gives only the type; see "ETS tables" in docs/erts-differences.md.
            Err(error(format!(
                "unsupported ets:info/2 item {}",
                atoms.name_of(atom)
            )))
        }
        _ => Ok(BifResult::badarg()),
    }
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

/// The most objects that one turn of a match reads before it can trap
/// (`MAX_SELECT_CHUNK_ITERATIONS`, `erl_db_hash.c:2161`).
const MATCH_ITERATIONS: i64 = 1000;

/// Run `ets:match(Table, Pattern)` or, with `Some(Limit)`,
/// `ets:match(Table, Pattern, Limit)`, as `ets_match_2` and `ets_match_3`
/// (`erl_db.c:3771-3818`) do with the match specification
/// `[{Pattern, [], ['$$']}]`. The checks keep the ERTS order:
///
/// 1. A table that does not exist or that the caller cannot read raises
///    `badarg` (`erl_db.c:769-912`).
/// 2. A limit that is not a small integer above 0 raises `badarg`
///    (`erl_db.c:3803-3807`).
/// 3. A pattern that does not compile raises `badarg`
///    (`erl_db_hash.c:3296-3310`): see [`Pattern::compile`].
///
/// Then [`match_turn`] reads the table.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_objects(&mut context, &table, &test_atom("$1"), None)?;
/// assert!(matches!(result, BifResult::Value(_)));
/// assert!(matches!(match_objects(&mut context, &table, &test_atom("_"), Some(&Term::Integer(0)))?, BifResult::Raise { .. }));
/// ```
///
/// # Errors
///
/// None in the first turn. See [`match_turn`].
pub(super) fn match_objects(
    context: &mut BifContext<'_>,
    table: &Term,
    pattern: &Term,
    limit: Option<&Term>,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let atoms = &context.scheduler.atoms;
    let Some(id) = context.scheduler.ets.find(atoms, table).map(|(id, _)| id) else {
        return Ok(BifResult::badarg());
    };
    let Some(table) =
        context
            .scheduler
            .ets
            .table(atoms, &Term::Reference(id), caller, Operation::Read)
    else {
        return Ok(BifResult::badarg());
    };
    let chunk = match limit.map(Term::small_integer) {
        None => 0,
        Some(Some(chunk)) if chunk > 0 => chunk as usize,
        Some(_) => return Ok(BifResult::badarg()),
    };
    let Some(compiled) = Pattern::compile(atoms, pattern) else {
        return Ok(BifResult::badarg());
    };
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
                state.read(&compiled, table.chain(bucket), &mut iterations);
                return match_end(context, id, state, -1, iterations);
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
    match_turn(context, id, pattern, &compiled, 0, state)
}

/// Run the trap export `ets:select_trap/7` of a match: the table reference,
/// the pattern, the next bucket, the limit or 0, the matches so far, the
/// number of matches, and the change count of the table. ERTS traps to
/// `ets:select_trap/1` with a tuple that holds a magic reference to the
/// compiled pattern (`erl_db.c:3883-3930`; `erl_db_hash.c:2264-2307`), and
/// PBeam compiles the pattern again in each turn. The compile reads only
/// the pattern, so its work does not grow with the table.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_trap(&mut context, id, &pattern, 300, MatchState::new(0, 1000))?;
/// assert!(matches!(result, BifResult::Value(_) | BifResult::TrapBif { .. }));
/// ```
///
/// # Errors
///
/// See [`match_turn`].
pub(super) fn match_trap(
    context: &mut BifContext<'_>,
    id: Reference,
    pattern: &Term,
    bucket: usize,
    state: MatchState,
) -> Result<BifResult, BeamError> {
    let caller = context.scheduler.current_process_mut().id;
    let atoms = &context.scheduler.atoms;
    let Some(table) =
        context
            .scheduler
            .ets
            .table(atoms, &Term::Reference(id), caller, Operation::Read)
    else {
        return Ok(BifResult::badarg());
    };
    if table.writes() != state.writes {
        // ponytail: ERTS fixes the table; see "ETS tables" in docs/erts-differences.md.
        return Err(error(
            "unsupported ets:match/2,3 on a table that changed while the match trapped",
        ));
    }
    let compiled = Pattern::compile(atoms, pattern).expect("the first turn compiled the pattern");
    match_turn(context, id, pattern, &compiled, bucket, state)
}

/// The state of a match across its turns.
pub(super) struct MatchState {
    /// The limit of `ets:match/3`, or 0 for `ets:match/2`.
    pub(super) chunk: usize,
    /// The list of the matches so far, the last match first, as
    /// `select_chunk_on_match_res` (`erl_db_hash.c:2174-2185`) puts each
    /// match before the earlier ones.
    pub(super) matches: Term,
    /// The number of matches so far.
    pub(super) got: usize,
    /// The change count of the table in the first turn.
    pub(super) writes: u64,
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
        }
    }

    /// Match each object of `objects` with `pattern` and use one iteration
    /// for each object (`erl_db_hash.c:1900-1916`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// state.read(&pattern, table.chain(0), &mut iterations);
    /// ```
    fn read<'a>(
        &mut self,
        pattern: &Pattern,
        objects: impl Iterator<Item = &'a [Term]>,
        iterations: &mut i64,
    ) {
        for object in objects {
            if let Some(found) = pattern.run(object) {
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
/// `ets:match/3`. Else, when the turn used its 1000 iterations, the call
/// uses all reductions and traps to `ets:select_trap/7` with the next
/// bucket ([`match_trap`]). A bucket is never split across turns, so a
/// trap keeps the position and no turn reads a bucket again.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_turn(&mut context, id, &pattern, &compiled, 0, MatchState::new(0, 0))?;
/// assert!(matches!(result, BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// None. The result type is the one of the BIF bodies.
fn match_turn(
    context: &mut BifContext<'_>,
    id: Reference,
    pattern: &Term,
    compiled: &Pattern,
    mut bucket: usize,
    mut state: MatchState,
) -> Result<BifResult, BeamError> {
    let table = context
        .scheduler
        .ets
        .find(&context.scheduler.atoms, &Term::Reference(id));
    let (_, table) = table.expect("the caller found the table");
    let mut iterations = MATCH_ITERATIONS;
    loop {
        state.read(compiled, table.chain(bucket), &mut iterations);
        bucket += 1;
        if bucket == table.bucket_count() {
            return match_end(context, id, state, -1, iterations);
        }
        if state.chunk > 0 && state.got >= state.chunk {
            return match_end(context, id, state, bucket as i64, iterations);
        }
        if iterations <= 0 {
            context
                .scheduler
                .current_process_mut()
                .bump_all_reductions();
            return Ok(BifResult::TrapBif {
                module: "ets",
                function: "select_trap",
                arguments: vec![
                    Term::Reference(id),
                    pattern.clone(),
                    Term::Integer(bucket as i64),
                    Term::Integer(state.chunk as i64),
                    state.matches,
                    Term::Integer(state.got as i64),
                    Term::Integer(state.writes as i64),
                ],
            });
        }
    }
}

/// End a match whose next bucket is `next`, or -1 after the last bucket,
/// with `iterations` left, as `select_chunk_on_loop_ended` and
/// `select_chunk_continue_on_loop_ended` (`erl_db_hash.c:2187-2262`,
/// `2364-2409`) do. The call uses one reduction for each iteration. Then:
///
/// - `ets:match/2` returns the matches.
/// - `ets:match/3` keeps the first matches of the list over the limit as
///   the rest. It returns the other matches and the continuation
///   `{Table, Next, Limit, Program, Rest, RestSize}` when there is a rest
///   or a next bucket, else `{Matches, '$end_of_table'}`, or
///   `'$end_of_table'` with no match. ERTS puts a magic reference to the
///   compiled pattern in `Program`, and PBeam puts a new reference there.
///
/// # Examples
///
/// ```rust,ignore
/// let result = match_end(&mut context, id, MatchState::new(1, 0), -1, 1000)?;
/// assert!(matches!(result, BifResult::Value(value) if value == Term::static_atom("$end_of_table")));
/// ```
///
/// # Errors
///
/// Return a runtime error when the references of the run are used up.
fn match_end(
    context: &mut BifContext<'_>,
    id: Reference,
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
    if !rest.is_empty() || next >= 0 {
        let program = context.scheduler.make_ref()?;
        let size = rest.len() as i64;
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
