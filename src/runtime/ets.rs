//! ETS tables of the node, as the ERTS table store (`erl_db.c`). All
//! processes of a run share one store.

pub(super) mod pattern;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};

use crate::term::atom_hash;
use crate::term::{AtomTable, LiveBytes, ProcessId, Reference, Term, internal_hash};

/// The number of slots of the first segment of a hash table, which a new
/// table has (`FIRST_SEGSZ`, `erl_db_hash.c:164-165`).
const FIRST_SEGMENT_SLOTS: usize = 256;
/// The number of slots of each later segment (`EXT_SEGSZ`,
/// `erl_db_hash.c:169-170`).
const SEGMENT_SLOTS: usize = 2048;
/// The reductions of a segment that the exit of the owner frees, without its
/// objects (`EXT_SEGSZ/64`, `erl_db_hash.c:3141`).
const SEGMENT_REDUCTIONS: i32 = (SEGMENT_SLOTS / 64) as i32;
/// The fewest slots with which a table can shrink: one more segment than two
/// segments (`FIRST_SEGSZ + 2*EXT_SEGSZ`, `erl_db_hash.c:3382`).
const SHRINK_SLOTS: usize = FIRST_SEGMENT_SLOTS + 2 * SEGMENT_SLOTS;
/// The most buckets that one grow or shrink call joins or splits
/// (`loop_limit`, `erl_db_hash.c:3656`, `3752`).
const RESIZE_STEPS: usize = 5;
/// The marker of a free slot of the key index.
const EMPTY: usize = usize::MAX;
/// The number of slots of the first key index.
const INITIAL_INDEX_SLOTS: usize = 8;

/// The ETS tables of the node. ERTS identifies each table by a table
/// reference that `make_btid` (`erl_db.c:2811`) gives it, also a named
/// table, and finds a named table by its atom in the meta name table. An
/// atom name is unique in one atom table, so its text identifies it, as in
/// `Scheduler::names`. PBeam supports `set`, `bag`, and `duplicate_bag`
/// tables.
#[derive(Debug, Default)]
pub(super) struct EtsTables {
    /// Each live table by its table reference.
    tables: HashMap<Reference, Table>,
    /// The table reference of each named table by its name.
    names: HashMap<String, Reference>,
    /// Each deleted table whose free switched out or trapped, by its table
    /// reference: the exit of the owner (`erl_db.c:5033-5041`, `5133-5143`)
    /// or `ets:delete/1` (`erl_db.c:3064-3077`). No process finds the table
    /// by its name or its reference at that time, but it stays in the owned
    /// list of its owner until each segment is free (`erl_db.c:5405-5426`).
    freeing: HashMap<Reference, Freeing>,
    /// The count of the tables that went into an owned list
    /// (`save_owned_table`, `erl_db.c:628-650`), so the next table gets
    /// this position, at the end of the list of its owner.
    saved: u64,
    /// The compiled match specification of each `ets:select/1`
    /// continuation, by the reference that the continuation holds in place
    /// of the ERTS magic reference (`ets_db_make_match_prog_ref`,
    /// `erl_db_hash.c:2103`, `2229`). Each entry also holds the table
    /// reference, so the delete of the table drops its programs.
    programs: HashMap<Reference, (Reference, pattern::MatchSpec)>,
}

/// A deleted table and the count of its segments that are not free yet. ERTS
/// frees the segments from the top segment down and keeps the position in
/// the table (`erl_db_hash.c:3140-3149`), so a later turn continues at the
/// next segment and does not walk a free segment again.
#[derive(Debug)]
pub(super) struct Freeing {
    /// The table. Its free segments hold no object.
    table: Table,
    /// The count of the segments that are not free, so the top segment that
    /// is not free is `segments - 1`. An `ordered_set` table frees its
    /// objects one by one, so the count is not used.
    segments: usize,
    /// The bucket where the free of the objects of an `ordered_set` table
    /// continues, as the work stack of `do_free_tree_continue` remembers the
    /// nodes (`erl_db_tree.c:2877-2904`).
    bucket: usize,
    /// The next key of the bucket `bucket` whose object is not free, or
    /// `EMPTY` when the walk reads the next bucket.
    next: usize,
}

/// One ETS hash table: a `set`, a `bag`, or a `duplicate_bag` table.
#[derive(Debug)]
pub(super) struct Table {
    /// The process that owns the table, as `tb->common.owner`.
    owner: ProcessId,
    /// The table type, as `tb->common.status & (DB_SET | DB_BAG |
    /// DB_DUPLICATE_BAG)`.
    table_type: TableType,
    /// Which other processes can read and write the table.
    access: Access,
    /// The 1-based key position, as the C `int` `tb->common.keypos`
    /// (`erl_db_util.h:346`). `ets:new/2` accepts each small integer above
    /// 0, and the C assignment keeps only its low 32 bits, so the value can
    /// be 0 or negative.
    keypos: i32,
    /// The object of each allocated slot, or `None` when the slot is free. A
    /// slot id never changes while the table is live, so the key index stores
    /// it and a delete moves no other object. A traversal reads the live
    /// slots in slot order and skips the free ones.
    objects: Vec<Option<Vec<Term>>>,
    /// The free slots, last in first out, so a later insert reuses them.
    free: Vec<usize>,
    /// For each allocated slot, the slot of the next older object with the
    /// same key, or `EMPTY` for the oldest object of its key. A `set` table
    /// has one object for each key, so each live slot holds `EMPTY`. The
    /// objects of one key are a chain from the newest to the oldest object,
    /// as the ERTS hash chain of a bag puts a new object before the other
    /// objects of its key (`erl_db_hash.c:1447-1452`).
    older: Vec<usize>,
    /// The stored key hash of each key to the slot of its newest object.
    /// Look up a key through this index, not by a walk of the slots.
    index: KeyIndex,
    /// The slot of the newest object of the first key of each active bucket
    /// of the ERTS hash table, or `EMPTY` for an empty bucket. The bucket
    /// chain (`BUCKET`, `erl_db_hash.c:186-190`) holds the keys in this
    /// order, and the objects of one key are contiguous from the newest to
    /// the oldest. A traversal reads the buckets from 0 up and each chain
    /// in order (`match_traverse`, `erl_db_hash.c:1835-1971`).
    heads: Vec<usize>,
    /// For the newest object of each key, the slot of the newest object of
    /// the next key in its bucket chain, or `EMPTY` for the last key. Each
    /// other slot holds `EMPTY`.
    next: Vec<usize>,
    /// The number of changes to the objects of the table. A trapped match
    /// keeps it, so its next turn can see a change that came between the
    /// turns.
    writes: u64,
    /// The number of active buckets of the ERTS hash table (`nactive`). It
    /// gives the number of slots and the slot of each key, so the exit of
    /// the owner frees the same segments with the same objects as ERTS. A
    /// new table has 256. ERTS splits one bucket when an insert makes the
    /// objects more than the buckets (`GROW_LIMIT`, `erl_db_hash.c:153`,
    /// `1349-1354`, `3648-3740`), and joins buckets when a delete makes the
    /// objects fewer than the shrink limit (`erl_db_hash.c:757-762`,
    /// `3350-3392`, `3743-3818`). With one scheduler, no table has fine
    /// locks (`erl_db.c:2656-2657`; `erl_db_util.h:388-389`), so the object
    /// count of these decisions is exact.
    buckets: usize,
    /// The position of the table in the owned list of its owner, from
    /// `EtsTables::saved`. ERTS appends a table to the list when
    /// `ets:new/2` makes it and when `ets:delete/1` of another process
    /// takes it (`save_owned_table`, `erl_db.c:628-650`, `3031-3051`).
    owned: u64,
    /// The atom that `ets:new/2` gave the table, also when the table is
    /// not named. `ets:info(Table, name)` gives it (`table_info`,
    /// `erl_db.c:5504-5505`).
    name: String,
    /// The `write_concurrency` option, as `ets:info(Table,
    /// write_concurrency)` gives it (`table_info`, `erl_db.c:5487-5501`).
    write_concurrency: Term,
    /// The `read_concurrency` option (`erl_db.c:5502-5503`).
    read_concurrency: bool,
    /// The `decentralized_counters` option (`erl_db.c:5516-5517`).
    decentralized_counters: bool,
    /// The `compressed` option (`erl_db.c:5512-5513`).
    compressed: bool,
    /// The heir that `ets:new/2` accepted and that is the owner, as
    /// `ets:info(Table, heir)` gives it (`table_info`, `erl_db.c:5478-5479`).
    heir: Option<ProcessId>,
    /// The list insert that took the table and yields between its objects
    /// (`tb->common.status & DB_BUSY`, `erl_db.c:2077-2107`, `2310-2319`),
    /// else `None`. Each process that uses the table while it is busy
    /// completes the insert first (`handle_lacking_permission`,
    /// `erl_db.c:769-798`).
    pub(super) busy: Option<Busy>,
}

/// A list insert that took a table and did not finish in its turn, as the
/// YCF state that `tb->common.continuation_ctx` points at
/// (`erl_db.c:2091-2107`, `2310-2319`). The table owns the copies of the
/// objects, so each process that uses the table can run the steps.
#[derive(Debug)]
pub(super) struct Busy {
    /// The process that started the insert. It gets the result of the
    /// completed insert (`erl_db.c:2223-2244`).
    pub(super) owner: ProcessId,
    /// The object of each position of the list, as the copies that
    /// `ets_insert_2_list_copy_term_list` (`erl_db.c:1876-1907`) makes.
    objects: Vec<Vec<Term>>,
    /// True when the call is `ets:insert_new/2` and the walk for a key
    /// that the table holds did not finish (`erl_db.c:1909-1925`).
    walk: bool,
    /// The number of objects that the walk checked.
    walked: usize,
    /// True when the walk found a key that the table holds.
    found: bool,
    /// The number of objects that are in the table, or whose copy went
    /// away after the walk found a key.
    inserted: usize,
    /// True when the loop unit of the step of the object `inserted` is
    /// charged and the object is not in the table yet, as the counter of
    /// the YCF loop can yield between the two (`erl_db.c:1936-1940`).
    looped: bool,
    /// The walk of each key of a `bag` insert ([`Table::insert_all`]),
    /// which the steps share ([`Table::insert_bag`]).
    walks: HashMap<u64, Vec<BagWalk>>,
}

impl Busy {
    /// Return the busy state of a list insert of the BIF `bif` over the
    /// objects `objects`, which the process `owner` started, as the YCF
    /// state of `ets_insert_2_list` starts it: the copies of the objects
    /// and the walk of `ets:insert_new/2` from its first object
    /// (`erl_db.c:1876-1907`, `2072-2074`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let busy = Busy::new(ProcessId(0), vec![vec![test_atom("k")]], false);
    /// assert_eq!(busy.walked, 0);
    /// ```
    pub(super) fn new(owner: ProcessId, objects: Vec<Vec<Term>>, walk: bool) -> Busy {
        Busy {
            owner,
            objects,
            walk,
            walked: 0,
            found: false,
            inserted: 0,
            looped: false,
            walks: HashMap::new(),
        }
    }

    /// Return the result of the insert, and end the busy state of the table
    /// when each object is in it or its copy went away, as
    /// `ets_insert_2_list_continuation` sets `ctx->return_value` and
    /// uninstalls the continuation (`erl_db.c:2180-2195`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut busy = Busy::new(ProcessId(0), Vec::new(), false);
    /// assert_eq!(busy.end(), Some(Term::static_atom("true")));
    /// ```
    fn end(&mut self) -> Option<Term> {
        let ended =
            self.inserted == self.objects.len() && !self.looped && (self.found || !self.walk);
        ended.then(|| Term::static_atom(if self.found { "false" } else { "true" }))
    }
}

/// The type of a hash table: the part of `tb->common.status` that
/// `ets:new/2` sets from its type option (`erl_db.c:2599-2611`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TableType {
    /// `DB_SET`: one object for each key.
    Set,
    /// `DB_BAG`: objects with one key that are not exactly equal.
    Bag,
    /// `DB_DUPLICATE_BAG`: objects with one key, also exactly equal ones.
    DuplicateBag,
    /// `DB_ORDERED_SET`: one object for each key. PBeam keeps the hash
    /// structure, so the results and the reductions of the single-key
    /// operations match ERTS; an operation whose result depends on the key
    /// order is a clear runtime error (see "ETS tables" in
    /// `docs/erts-differences.md`).
    OrderedSet,
}

impl TableType {
    /// Return the name of the type, as `ets:info(Table, type)` gives it
    /// (`table_info`, `erl_db.c:5457-5469`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(TableType::DuplicateBag.name(), "duplicate_bag");
    /// ```
    pub(super) fn name(self) -> &'static str {
        match self {
            TableType::Set => "set",
            TableType::Bag => "bag",
            TableType::DuplicateBag => "duplicate_bag",
            TableType::OrderedSet => "ordered_set",
        }
    }
}

/// The access mode of a table: `tb->common.status & (DB_PUBLIC |
/// DB_PROTECTED | DB_PRIVATE)`.
#[derive(Debug, Clone, Copy)]
pub(super) enum Access {
    /// Each process can read and write the table.
    Public,
    /// Each process can read the table. Only the owner can write it.
    Protected,
    /// Only the owner can read and write the table.
    Private,
}

/// What a BIF does with a table: the `what` argument of `db_get_table`
/// (`erl_db.c:800-912`).
#[derive(Debug, Clone, Copy)]
pub(super) enum Operation {
    /// `DB_READ`, which a `public` or a `protected` table allows
    /// (`erl_db_util.h:449`).
    Read,
    /// `DB_WRITE`, which only a `public` table allows (`erl_db_util.h:450`).
    Write,
}

impl EtsTables {
    /// Return whether a live table has the name `name`. A name that is in
    /// use makes `insert_named_tab` (`erl_db.c:1000-1047`) fail, and
    /// `ets:new/2` then raises `badarg` (`erl_db.c:2821-2831`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// assert!(!tables.name_in_use("ac_tab"));
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Public, 1, TableType::Set));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn name_in_use(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Add the table `table` with the reference `id`. A named table also
    /// gets its name. The caller makes sure that the name is not in use.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), None, Table::new(ProcessId(0), Access::Protected, 1, TableType::Set));
    /// tables.create(Reference(1), Some("ac_tab"), Table::new(ProcessId(0), Access::Public, 1, TableType::Set));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn create(&mut self, id: Reference, name: Option<&str>, mut table: Table) {
        table.owned = self.save_owned();
        self.tables.insert(id, table);
        if let Some(name) = name {
            self.names.insert(name.to_owned(), id);
        }
    }

    /// Count the heap storage of the tables and the guest terms that they
    /// hold: the slot array of the objects, the free-slot array, and the
    /// element array of each stored object, plus each element of each stored
    /// object, including the key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k"), Term::tuple(vec![Term::Integer(1)])]);
    /// tables.create(Reference(1), None, table);
    /// let mut live = LiveBytes::new();
    /// tables.account_terms(&mut live);
    /// assert!(live.bytes() > 0);
    /// ```
    pub(super) fn account_terms(&self, live: &mut LiveBytes) {
        let freeing = self.freeing.values().map(|freeing| &freeing.table);
        for table in self.tables.values().chain(freeing) {
            live.add(table.allocated_bytes());
            let stored = table.objects.iter().flatten();
            let copies = table
                .busy
                .as_ref()
                .into_iter()
                .flat_map(|busy| &busy.objects);
            for object in stored.chain(copies) {
                for term in object {
                    live.term(term);
                }
            }
            for walk in table
                .busy
                .as_ref()
                .into_iter()
                .flat_map(|busy| busy.walks.values())
                .flatten()
            {
                live.term(&walk.key);
            }
        }
    }

    /// Return the table reference of the live table with the name `name`, or
    /// `None` for a free name. `ets:whereis/1` (`erl_db.c:2849-2872`) asks
    /// only for `DB_INFO` (`erl_db_util.h:451`), which each process has, so
    /// the caller and the access mode do not matter.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("logger"), Table::new(ProcessId(0), Access::Private, 1, TableType::Set));
    /// assert_eq!(tables.whereis("logger"), Some(Reference(0)));
    /// assert_eq!(tables.whereis("missing"), None);
    /// ```
    pub(super) fn whereis(&self, name: &str) -> Option<Reference> {
        self.names.get(name).copied()
    }

    /// Return whether the table is a named table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(!tables.named(Reference(0)));
    /// ```
    pub(super) fn named(&self, id: Reference) -> bool {
        self.names.values().any(|named| *named == id)
    }

    /// Return the table that `id` identifies, when the process `caller` can
    /// do `operation` on it, as `db_get_table` (`erl_db.c:800-912`) does. An
    /// atom finds a named table, and a reference finds its table. Return
    /// `None` for each other term, a free name, a deleted table, and a
    /// process that is not the owner and that the access mode does not
    /// allow (`handle_lacking_permission`, `erl_db.c:769-798`). The ETS BIFs
    /// then raise `badarg`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Protected, 1, TableType::Set));
    /// let name = Term::static_atom("ac_tab");
    /// assert!(tables.table(&name, ProcessId(1), Operation::Read).is_some());
    /// assert!(tables.table(&name, ProcessId(1), Operation::Write).is_none());
    /// assert!(tables.table(&name, ProcessId(0), Operation::Write).is_some());
    /// assert!(tables.table(&Term::Integer(1), ProcessId(0), Operation::Read).is_none());
    /// ```
    pub(super) fn table(
        &mut self,
        atoms: &AtomTable,
        id: &Term,
        caller: ProcessId,
        operation: Operation,
    ) -> Option<&mut Table> {
        let (id, _) = self.find(atoms, id)?;
        let table = self.tables.get_mut(&id)?;
        let allowed = table.owner == caller
            || matches!(
                (table.access, operation),
                (Access::Public, _) | (Access::Protected, Operation::Read)
            );
        allowed.then_some(table)
    }

    /// Return the table reference and the table that `id` identifies, with
    /// no access check, as `db_get_table` with `DB_READ_TBL_STRUCT` does for
    /// a list insert (`erl_db.c:845-857`; `erl_db_util.h:452`). An atom finds
    /// a named table, and a reference finds its table. Return `None` for each
    /// other term, a free name, and a deleted table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Private, 1, TableType::Set));
    /// let atoms = AtomTable::new();
    /// assert!(tables.find(&atoms, &Term::static_atom("ac_tab")).is_some_and(|(id, _)| id == Reference(0)));
    /// assert!(tables.find(&atoms, &Term::Reference(Reference(1))).is_none());
    /// assert!(tables.find(&atoms, &Term::Integer(1)).is_none());
    /// ```
    pub(super) fn find(&self, atoms: &AtomTable, id: &Term) -> Option<(Reference, &Table)> {
        let id = match id {
            Term::Atom(name) => *self.names.get(atoms.name_of(name))?,
            Term::Reference(id) => *id,
            _ => return None,
        };
        self.tables.get(&id).map(|table| (id, table))
    }

    /// Return the table that `id` identifies when a list insert that took
    /// it did not finish, else `None`, as `db_get_table` gives a table
    /// with `DB_BUSY` to `handle_lacking_permission` (`erl_db.c:769-798`).
    /// A process that uses the table then completes the insert first.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.busy = Some(Busy::new(ProcessId(0), Vec::new(), false));
    /// tables.create(Reference(0), None, table);
    /// assert!(tables.busy(&atoms, &Term::Reference(Reference(0))).is_some());
    /// ```
    pub(super) fn busy(&mut self, atoms: &AtomTable, id: &Term) -> Option<&mut Table> {
        let (id, busy) = {
            let (id, table) = self.find(atoms, id)?;
            (id, table.busy.is_some())
        };
        busy.then(|| self.tables.get_mut(&id)).flatten()
    }

    /// Remember the compiled match specification `spec` under the reference
    /// `program` of a continuation of the table `table`, as ERTS keeps the
    /// compiled program in a magic reference (`erl_db_hash.c:2103`,
    /// `2229`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// let spec = MatchSpec::compile(&atoms, &Term::nil()).unwrap();
    /// tables.remember_program(Reference(0), Reference(1), spec);
    /// assert!(tables.program(Reference(1)).is_some());
    /// ```
    pub(super) fn remember_program(
        &mut self,
        table: Reference,
        program: Reference,
        spec: pattern::MatchSpec,
    ) {
        self.programs.insert(program, (table, spec));
    }

    /// Return the compiled match specification of the continuation program
    /// `program`, or `None` when no continuation of a live table holds it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(EtsTables::default().program(Reference(0)).is_none());
    /// ```
    pub(super) fn program(&self, program: Reference) -> Option<&pattern::MatchSpec> {
        self.programs.get(&program).map(|(_, spec)| spec)
    }

    /// Return the live table with the reference `id`, with no access
    /// check, as [`EtsTables::find`] finds it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), None, Table::new(ProcessId(0), Access::Public, 1, TableType::Set));
    /// assert!(tables.table_by_id(Reference(0)).is_some());
    /// ```
    #[cfg(test)]
    pub(super) fn table_by_id(&mut self, id: Reference) -> Option<&mut Table> {
        self.tables.get_mut(&id)
    }

    /// Delete the tables that the process `owner` owns, as
    /// `erts_db_process_exiting` (`erl_db.c:5031-5144`) does when the owner
    /// exits, with `reds` reductions left in the turn. Return the reductions
    /// that are left when each table is free, or `None` when the exit must
    /// switch out and call again in its next turn.
    ///
    /// The tables go in the order of the owned list of `owner`, from its
    /// head (`erl_db.c:5055-5096`): see `Table::owned`. A table that a
    /// trapped `ets:delete/1` of `owner` frees keeps its position in the
    /// list, so an older table goes first. A live table first loses its name and its reference,
    /// so no other process finds it, and an insert that traps on it then
    /// raises `badarg` (`erl_db.c:5082-5094`). Then each segment, from the
    /// top segment down, uses 32 reductions and 1 for each object in it
    /// (`erl_db_hash.c:3122-3149`, `3468-3530`). When the reductions are
    /// less than 0 after a segment, or 0 after a table, the exit switches
    /// out (`erl_db.c:5098-5104`, `5131`). PBeam has no heir and no table
    /// fixation, so no table goes to another process and no fixation costs.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(1), Access::Public, 1, TableType::Set));
    /// assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(68));
    /// assert!(!tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn delete_owned(&mut self, owner: ProcessId, mut reds: i32) -> Option<i32> {
        loop {
            let live = (self.tables.iter()).map(|(id, table)| (*id, table));
            let freeing = (self.freeing.iter()).map(|(id, freeing)| (*id, &freeing.table));
            let Some((_, id)) = live
                .chain(freeing)
                .filter(|(_, table)| table.owner == owner)
                .map(|(id, table)| (table.owned, id))
                .min()
            else {
                return Some(reds);
            };
            if let Some(table) = self.tables.remove(&id) {
                self.names.retain(|_, named| *named != id);
                self.programs.retain(|_, (table, _)| *table != id);
                self.freeing.insert(id, Freeing::new(table));
            }
            reds = self.free_table(id, reds)?;
            if reds == 0 {
                return None;
            }
        }
    }

    /// Delete the table `id` and its name, as `ets:delete/1`
    /// (`erl_db.c:3053-3061`) does before it frees the table, and keep the
    /// table under the process `caller`, which then owns the table
    /// (`erl_db.c:3031-3051`). A table of another owner goes to the end of
    /// the owned list of `caller` (`save_owned_table`, `erl_db.c:628-650`).
    /// [`EtsTables::free_segments`]
    /// frees them. The caller makes sure that the table is live.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(1), Access::Public, 1, TableType::Set));
    /// tables.delete_table(Reference(0), ProcessId(2));
    /// assert!(!tables.name_in_use("ac_tab"));
    /// assert_eq!(tables.free_segments(ProcessId(2), 100), Some(68));
    /// ```
    pub(super) fn delete_table(&mut self, id: Reference, caller: ProcessId) {
        let mut table = self.tables.remove(&id).expect("a deleted table is live");
        self.names.retain(|_, named| *named != id);
        self.programs.retain(|_, (table, _)| *table != id);
        if table.owner != caller {
            table.owner = caller;
            table.owned = self.save_owned();
        }
        self.freeing.insert(id, Freeing::new(table));
    }

    /// Return the position of the next table that goes into an owned list,
    /// at the end of the list (`save_owned_table`, `erl_db.c:628-650`).
    fn save_owned(&mut self) -> u64 {
        self.saved += 1;
        self.saved
    }

    /// Free the segments of the table that the trapped `ets:delete/1` of
    /// `owner` deleted. A process that does not exit traps in one delete
    /// only, so it frees one table. See [`EtsTables::free_table`].
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), None, Table::new(ProcessId(1), Access::Public, 1, TableType::Set));
    /// tables.delete_table(Reference(0), ProcessId(1));
    /// assert_eq!(tables.free_segments(ProcessId(1), 100), Some(68));
    /// ```
    pub(super) fn free_segments(&mut self, owner: ProcessId, reds: i32) -> Option<i32> {
        let id = (self.freeing.iter())
            .find(|(_, freeing)| freeing.table.owner == owner)
            .map(|(id, _)| *id)?;
        self.free_table(id, reds)
    }

    /// Free the segments that the deleted table `id` keeps, from the top
    /// segment down, with `reds` reductions left, as
    /// `db_free_table_continue_hash` (`erl_db_hash.c:3140-3149`) does: each
    /// segment frees its objects and uses its reductions, and the work stops
    /// when the reductions are less than 0 after a segment. Return the reductions that are left
    /// when each segment is free, or `None` when segments remain.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), None, Table::new(ProcessId(1), Access::Public, 1, TableType::Set));
    /// tables.delete_table(Reference(0), ProcessId(1));
    /// assert_eq!(tables.free_table(Reference(0), 10), None);
    /// assert_eq!(tables.free_table(Reference(0), 100), Some(100));
    /// ```
    fn free_table(&mut self, id: Reference, mut reds: i32) -> Option<i32> {
        let freeing = self.freeing.get_mut(&id)?;
        while let Some(segment) = freeing.free_segment(reds) {
            reds -= segment;
            if reds < 0 {
                return None;
            }
        }
        self.freeing.remove(&id);
        Some(reds)
    }
}

impl Freeing {
    /// Return the free state of the deleted table `table`, with each of its
    /// segments not free yet.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let freeing = Freeing::new(Table::new(ProcessId(1), Access::Public, 1, TableType::Set));
    /// assert_eq!(freeing.segments, 1);
    /// ```
    fn new(table: Table) -> Freeing {
        let segments = if table.table_type == TableType::OrderedSet {
            1
        } else {
            segment(table.segment_slots() - 1) + 1
        };
        Freeing {
            table,
            segments,
            bucket: 0,
            next: EMPTY,
        }
    }

    /// Free the objects of the table and return their reductions: of the
    /// top segment that is not free yet of a hash table, with 32 for the
    /// segment and 1 for each object in it (`erl_db_hash.c:3141`,
    /// `3468-3530`), or of an `ordered_set` table, with 1 for each object
    /// (`erl_db_tree.c:2877-2904`). Return `None` when each object is free.
    /// The walk reads only the bucket chains of the segment, so the work of
    /// one segment grows with its objects only.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// table.insert(&AtomTable::new(), vec![Term::Integer(1)]);
    /// let mut freeing = Freeing::new(table);
    /// assert_eq!(freeing.free_segment(0), Some(33));
    /// assert_eq!(freeing.free_segment(0), None);
    /// ```
    fn free_segment(&mut self, budget: i32) -> Option<i32> {
        if self.table.table_type == TableType::OrderedSet {
            return self.free_objects(budget);
        }
        self.segments = self.segments.checked_sub(1)?;
        let first = match self.segments {
            0 => 0,
            later => FIRST_SEGMENT_SLOTS + (later - 1) * SEGMENT_SLOTS,
        };
        let last = (FIRST_SEGMENT_SLOTS + self.segments * SEGMENT_SLOTS).min(self.table.buckets);
        let table = &mut self.table;
        let mut reds = SEGMENT_REDUCTIONS;
        for bucket in first..last {
            let mut key = std::mem::replace(&mut table.heads[bucket], EMPTY);
            while key != EMPTY {
                let mut slot = key;
                key = table.next[key];
                while slot != EMPTY {
                    table.objects[slot] = None;
                    slot = table.older[slot];
                    reds += 1;
                }
            }
        }
        Some(reds)
    }

    /// Free the objects of an `ordered_set` table, one more than the
    /// reductions `budget` of the turn, as `do_free_tree_continue` frees a
    /// node for each reduction and returns when the reductions are less
    /// than 0 (`erl_db_tree.c:2877-2904`). Return the number of objects that
    /// are free, or `None` when the table holds no object. The walk follows
    /// the bucket chains of the live keys, so it reads no slot of an object
    /// that an earlier delete freed, as the ERTS walk reads only the live
    /// nodes of the tree.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::OrderedSet);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1)]);
    /// table.insert(&atoms, vec![Term::Integer(2)]);
    /// let mut freeing = Freeing::new(table);
    /// assert_eq!(freeing.free_objects(0), Some(1));
    /// assert_eq!(freeing.free_objects(0), Some(1));
    /// assert_eq!(freeing.free_objects(0), None);
    /// ```
    fn free_objects(&mut self, budget: i32) -> Option<i32> {
        let limit = usize::try_from(budget.max(0))
            .unwrap_or(0)
            .saturating_add(1);
        let mut freed = 0;
        while freed < limit {
            if self.next == EMPTY {
                if self.bucket == self.table.buckets {
                    break;
                }
                let head = std::mem::replace(&mut self.table.heads[self.bucket], EMPTY);
                if head == EMPTY {
                    self.bucket += 1;
                    continue;
                }
                self.next = head;
            }
            let slot = self.next;
            self.next = self.table.next[slot];
            self.table.objects[slot] = None;
            freed += 1;
            if self.next == EMPTY {
                self.bucket += 1;
            }
        }
        (freed > 0).then(|| i32::try_from(freed).unwrap_or(i32::MAX))
    }
}

impl Table {
    /// Return an empty table of the type `table_type` that the process
    /// `owner` owns, with the access mode `access` and the truncated key
    /// position `keypos`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Protected, 1, TableType::Bag);
    /// assert_eq!(table.keypos(), 1);
    /// assert_eq!(table.table_type(), TableType::Bag);
    /// ```
    pub(super) fn new(
        owner: ProcessId,
        access: Access,
        keypos: i32,
        table_type: TableType,
    ) -> Table {
        Table {
            owner,
            table_type,
            access,
            keypos,
            objects: Vec::new(),
            free: Vec::new(),
            older: Vec::new(),
            index: KeyIndex::default(),
            heads: vec![EMPTY; FIRST_SEGMENT_SLOTS],
            next: Vec::new(),
            writes: 0,
            buckets: FIRST_SEGMENT_SLOTS,
            owned: 0,
            name: String::new(),
            write_concurrency: Term::static_atom("false"),
            read_concurrency: false,
            decentralized_counters: false,
            compressed: false,
            heir: None,
            busy: None,
        }
    }

    /// Return the key position, as the C `int` of ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 2, TableType::Set).keypos(), 2);
    /// ```
    pub(super) fn keypos(&self) -> i32 {
        self.keypos
    }

    /// Return the table type.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// assert_eq!(table.table_type(), TableType::Set);
    /// ```
    pub(super) fn table_type(&self) -> TableType {
        self.table_type
    }

    /// Return the atom that `ets:new/2` gave the table, as
    /// `ets:info(Table, name)` gives it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// table.set_name("t");
    /// assert_eq!(table.name(), "t");
    /// ```
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    /// Set the atom that `ets:new/2` gave the table.
    pub(super) fn set_name(&mut self, name: &str) {
        self.name = name.to_owned();
    }

    /// Return the owner.
    pub(super) fn owner(&self) -> ProcessId {
        self.owner
    }

    /// Return the access mode.
    pub(super) fn access(&self) -> Access {
        self.access
    }

    /// Return the `write_concurrency` option.
    pub(super) fn write_concurrency(&self) -> &Term {
        &self.write_concurrency
    }

    /// Return the `read_concurrency` option.
    pub(super) fn read_concurrency(&self) -> bool {
        self.read_concurrency
    }

    /// Return the `decentralized_counters` option.
    pub(super) fn decentralized_counters(&self) -> bool {
        self.decentralized_counters
    }

    /// Return the `compressed` option.
    pub(super) fn compressed(&self) -> bool {
        self.compressed
    }

    /// Return the heir, as `ets:info(Table, heir)` gives it.
    pub(super) fn heir(&self) -> Option<ProcessId> {
        self.heir
    }

    /// Set the heir that `ets:new/2` accepted and that is the owner.
    pub(super) fn set_heir(&mut self, heir: Option<ProcessId>) {
        self.heir = heir;
    }

    /// Set the options that `ets:info/2` gives. `ets:new/2` accepts them
    /// and keeps no other effect of them, as it does not compress objects
    /// and has one scheduler.
    pub(super) fn set_options(
        &mut self,
        write_concurrency: Term,
        read_concurrency: bool,
        decentralized_counters: bool,
        compressed: bool,
    ) {
        self.write_concurrency = write_concurrency;
        self.read_concurrency = read_concurrency;
        self.decentralized_counters = decentralized_counters;
        self.compressed = compressed;
    }

    /// Return the number of objects.
    pub(super) fn len(&self) -> usize {
        self.object_count()
    }

    /// Return the reductions of `ets:delete_all_objects/1`, as
    /// `db_free_table_continue_hash` charges them (`erl_db_hash.c:3122-3149`):
    /// 32 for each segment and 1 for each object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.delete_all_reductions(), 32);
    /// ```
    pub(super) fn delete_all_reductions(&self) -> usize {
        if self.table_type == TableType::OrderedSet {
            // The ERTS tree frees one node for each reduction
            // (`do_free_tree_continue`, `erl_db_tree.c:2877-2904`).
            return self.object_count();
        }
        self.segment_slots().div_ceil(SEGMENT_SLOTS) * SEGMENT_REDUCTIONS as usize
            + self.object_count()
    }

    /// Return the memory of the table in 64-bit words, as
    /// `ets:info(Table, memory)` counts it (`erl_db.c:5470-5475`). ERTS
    /// counts its own allocation, so only the unit is the same.
    pub(super) fn memory_words(&self) -> usize {
        self.allocated_bytes().div_ceil(std::mem::size_of::<u64>())
    }

    /// Return the number of elements of the object with the key `key`, or
    /// `None` when the table holds no such object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(table.object_arity(&atoms, &Term::static_atom("k")), Some(2));
    /// ```
    pub(super) fn object_arity(&self, atoms: &AtomTable, key: &Term) -> Option<usize> {
        let slot = self.key_slot(atoms, key)?;
        Some(self.object_of(slot).len())
    }

    /// Set the 1-based elements `updates` of the object with the key `key`,
    /// as `db_do_update_element` (`erl_db_hash.c` `db_do_update_element`)
    /// does. Return `false` when the table holds no such object. The caller
    /// checks each position.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert!(table.update_elements(&atoms, &Term::static_atom("k"), &[(2, Term::Integer(9))]));
    /// ```
    pub(super) fn update_elements(
        &mut self,
        atoms: &AtomTable,
        key: &Term,
        updates: &[(usize, Term)],
    ) -> bool {
        let Some(slot) = self.key_slot(atoms, key) else {
            return false;
        };
        let object = self.objects[slot]
            .as_mut()
            .expect("a live slot holds an object");
        for (position, value) in updates {
            object[*position - 1] = value.clone();
        }
        if !updates.is_empty() {
            self.writes += 1;
        }
        true
    }

    /// Return the next key after `key` in the order of a hash table, as
    /// `db_next_hash` (`erl_db_hash.c:1147-1200`) gives it: the key of the
    /// next live object in the chain of the bucket, then the chain of each
    /// later bucket. Return `None` for a key that the table does not hold,
    /// and `Some(None)` after the last key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// assert_eq!(table.next_key(&atoms, &Term::static_atom("k")), None);
    /// ```
    pub(super) fn next_key(&self, atoms: &AtomTable, key: &Term) -> Option<Option<Term>> {
        let hash = stored_hash(atoms, key);
        let newest = self.position(hash, key)?;
        let mut bucket = self.bucket(hash);
        let mut slot = self.next[newest];
        loop {
            if slot != EMPTY {
                return Some(Some(self.key(self.object_of(slot)).clone()));
            }
            bucket += 1;
            if bucket == self.buckets {
                return Some(None);
            }
            slot = self.heads[bucket];
        }
    }

    /// Remove all objects of the table, as `db_delete_all_objects_hash`
    /// (`erl_db_hash.c:4017-4037`) does: the table gets the buckets of a new
    /// table and keeps its name, reference, owner, and options.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.clear(), 1);
    /// assert_eq!(table.len(), 0);
    /// ```
    pub(super) fn clear(&mut self) -> usize {
        let count = self.object_count();
        self.objects.clear();
        self.free.clear();
        self.older.clear();
        self.index = KeyIndex::default();
        self.heads = vec![EMPTY; FIRST_SEGMENT_SLOTS];
        self.next.clear();
        self.buckets = FIRST_SEGMENT_SLOTS;
        self.writes += 1;
        count
    }

    /// Remove one object that is exactly equal to `object`, as the
    /// `select_delete` callback removes one object (`erl_db_hash.c:2622-2653`).
    /// Return whether the table held it. The caller shrinks the table after
    /// the traversal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::DuplicateBag);
    /// let atoms = AtomTable::new();
    /// let object = vec![Term::static_atom("k"), Term::Integer(1)];
    /// table.insert(&atoms, object.clone());
    /// assert!(table.delete_object(&atoms, &object));
    /// assert!(!table.delete_object(&atoms, &object));
    /// ```
    pub(super) fn delete_object(&mut self, atoms: &AtomTable, object: &[Term]) -> bool {
        let key = self.key(object).clone();
        let hash = stored_hash(atoms, &key);
        let Some(mut slot) = self.position(hash, &key) else {
            return false;
        };
        let mut previous = EMPTY;
        while slot != EMPTY {
            if self.object_of(slot) == object {
                let bucket = self.bucket(hash);
                let next = self.older[slot];
                if previous == EMPTY {
                    let after = self.next[slot];
                    if next == EMPTY {
                        self.index.remove(hash, slot);
                        self.link(bucket, self.key_before(bucket, slot), after);
                    } else {
                        // The key stays in the chain, and its new newest
                        // object takes the place of the removed one
                        // (`select_delete_on_match_res`, erl_db_hash.c:2643-2648).
                        self.index.replace(hash, slot, next);
                        self.next[next] = after;
                        self.link(bucket, self.key_before(bucket, slot), next);
                    }
                } else {
                    self.older[previous] = next;
                }
                self.objects[slot] = None;
                self.older[slot] = EMPTY;
                self.next[slot] = EMPTY;
                self.free.push(slot);
                self.writes += 1;
                return true;
            }
            previous = slot;
            slot = self.older[slot];
        }
        false
    }

    /// Join buckets after a `select_delete` traversal, as
    /// `select_delete_on_loop_ended` calls `try_shrink` once
    /// (`erl_db_hash.c:2677-2709`).
    pub(super) fn shrink(&mut self) {
        for _ in 0..RESIZE_STEPS {
            if self.object_count() >= self.shrink_limit() {
                break;
            }
            self.join();
        }
    }

    /// Add the object with the elements `object` to a `set` or a
    /// `duplicate_bag` table, as `db_put_hash` (`erl_db_hash.c:1384-1462`)
    /// does. Such an insert compares no object. A `bag` insert compares
    /// objects, so it goes through [`Table::insert_all`].
    ///
    /// - In a `set` table, an object with an exactly equal key replaces the
    ///   old object (`erl_db_hash.c:1396-1411`).
    /// - In a `duplicate_bag` table, the insert compares nothing.
    ///
    /// A new object goes before the other objects of its key, so a lookup
    /// that reads the chain from the end gives the insertion order
    /// (`erl_db_hash.c:1447-1452`). A new object that makes the objects more
    /// than the buckets splits one bucket (`erl_db_hash.c:1453-1461`). The
    /// buckets are never fewer than the objects, so one split is enough. The
    /// caller makes sure that the key position is at least 1 and at most
    /// the size of the object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::DuplicateBag);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k"), Term::Integer(1)]);
    /// table.insert(&atoms, vec![Term::static_atom("k"), Term::Integer(1)]);
    /// assert_eq!(table.objects(&atoms, &Term::static_atom("k")).len(), 2);
    /// ```
    pub(super) fn insert(&mut self, atoms: &AtomTable, object: Vec<Term>) {
        let (found, hash) = {
            let key = self.key(&object);
            let hash = stored_hash(atoms, key);
            (self.key_slot(atoms, key), hash)
        };
        let newest = match (found, self.table_type) {
            (Some(slot), TableType::Set | TableType::OrderedSet) => {
                let old = stored_hash(atoms, self.key(self.object_of(slot)));
                self.rekey(slot, old, hash);
                self.objects[slot] = Some(object);
                self.writes += 1;
                return;
            }
            (Some(newest), _) => newest,
            (None, _) => EMPTY,
        };
        self.add(atoms, hash, object, newest);
    }

    /// Run up to `units` units of the steps of the list insert that took
    /// this table, as the YCF loop of `ets_insert_2_list` does
    /// (`erl_db.c:1927-1949`, `2091-2107`): the walk of
    /// `ets:insert_new/2` with one unit for each object that it checks
    /// (`erl_db.c:1909-1925`), then either the removal of the copies with
    /// one unit for each object after the walk found a key
    /// (`erl_db.c:1863-1873`) or the insert of the objects with two units
    /// for each object. Each step ends between two objects, so the
    /// position of the work is in this table and the call traps between
    /// the steps. Return the units that the steps used, over the units of
    /// the budget that the steps may use, and the result when the insert
    /// ended: `true` for an insert and `false` for a walk that found a key.
    ///
    /// The caller makes sure that the key position is not 0.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.busy = Some(Busy::new(ProcessId(0), vec![vec![test_atom("k")]], false));
    /// let (units, result) = table.step_busy(&atoms, 8);
    /// assert_eq!((units, result), (2, Some(Term::static_atom("true"))));
    /// ```
    pub(super) fn step_busy(&mut self, atoms: &AtomTable, units: i64) -> (i64, Option<Term>) {
        let Some(mut busy) = self.busy.take() else {
            return (0, None);
        };
        let mut left = units;
        let mut exhausted = false;
        // The counter of ERTS can be 0 or less at the start of a step, and
        // the step runs then: `YCF_CONSUME_REDS` yields after it
        // (`erl_db.c:1939`, `2152-2176`).
        let mut first = true;
        if busy.walk {
            while busy.walked < busy.objects.len() && (first || left > 0) {
                first = false;
                // The counter consumes the loop unit of the step and yields
                // before the key of the object; a turn that yields there
                // keeps the unit charged (`erl_db.c:1916-1923`).
                if !busy.looped {
                    left -= 1;
                    busy.looped = true;
                    if left <= 0 {
                        break;
                    }
                }
                let key = &busy.objects[busy.walked][self.keypos as usize - 1];
                busy.looped = false;
                if self.object(atoms, key).is_some() {
                    busy.found = true;
                    break;
                }
                busy.walked += 1;
            }
            if busy.found || busy.walked == busy.objects.len() {
                busy.walk = false;
            }
        }
        if busy.walk {
            self.busy = Some(busy);
            return (units - left, None);
        }

        if busy.found {
            // The copy of each object goes away, one unit for each object
            // (`ets_insert_2_list_destroy_copied_dbterms`, `erl_db.c:1863-1873`).
            // The counter yields after the loop unit of a step and before
            // the copy goes away, as it does in the loop of the insert
            // (`erl_db.c:1936-1940`).
            while busy.inserted < busy.objects.len() && (first || left > 0) {
                first = false;
                if !busy.looped {
                    left -= 1;
                    busy.looped = true;
                    if left <= 0 {
                        exhausted = true;
                        break;
                    }
                }
                std::mem::take(&mut busy.objects[busy.inserted]);
                busy.looped = false;
                busy.inserted += 1;
            }
            exhausted = exhausted || left <= 0;
        } else {
            // The objects go into the table, one unit for the loop of the
            // step and one for the object, and one more for each object
            // that a `bag` insert compares (`erl_db_hash.c:1422-1442`). The
            // counter yields after the loop unit and before the object, and
            // after the insert of the object (`erl_db.c:1936-1940`), so a
            // turn can charge the loop unit of an object that it does not
            // put into the table.
            while busy.inserted < busy.objects.len() && (first || left > 0) {
                first = false;
                if !busy.looped {
                    left -= 1;
                    busy.looped = true;
                    if left <= 0 {
                        exhausted = true;
                        break;
                    }
                }
                // The copy of the object goes into the table in its step
                // (`db_put_dbterm`, `erl_db.c:1934-1940`). A `bag` insert
                // keeps the walk of each key for its other objects, so its
                // work stays that of one list insert.
                let object = std::mem::take(&mut busy.objects[busy.inserted]);
                let compared = if self.table_type == TableType::Bag {
                    self.insert_bag(atoms, object, &mut busy.walks)
                } else {
                    self.insert(atoms, object);
                    0
                };
                busy.looped = false;
                busy.inserted += 1;
                left -= 1 + compared as i64;
            }
            // A turn that uses the last of the budget ends the insert in
            // its next turn, as the trapped YCF call uninstalls the state
            // then (`erl_db.c:2180-2196`, `2297-2332`).
            exhausted = exhausted || left <= 0;
        }
        let result = if exhausted { None } else { busy.end() };
        if result.is_none() {
            self.busy = Some(busy);
        }
        (units - left, result)
    }

    /// Add each object of `objects` in list order, as a list insert puts
    /// them, and return the number of objects that the insert of each
    /// object compared, as one insert after another gives it.
    ///
    /// In a `bag` table, ERTS compares an object with each object of its
    /// key up to an exactly equal one, so a list of `n` objects of one key
    /// can do `n * (n - 1) / 2` compares (`erl_db_hash.c:1422-1442`). PBeam
    /// puts the whole list into the table at once (the BIF `insert_list`),
    /// so it gets each count with less work. For each key it keeps a
    /// [`BagWalk`]: the objects of the list that it added, and the objects
    /// of the key that were in the table, which it walks only as far as
    /// ERTS compares them. A hash of the whole object finds an exactly
    /// equal one. The expected work is then linear in the objects of the
    /// list and in the walked objects, and the counts stay the ERTS ones.
    /// Only objects with equal 64-bit hashes ([`object_hash`]) are
    /// compared, so the work is quadratic only when many different objects
    /// of one key have one hash. Each other table type compares nothing.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
    /// let atoms = AtomTable::new();
    /// let object = |value| vec![Term::Integer(1), Term::Integer(value)];
    /// table.insert_all(&atoms, vec![object(1)]);
    /// assert_eq!(table.insert_all(&atoms, vec![object(2), object(1), object(3)]), vec![1, 1, 2]);
    /// ```
    pub(super) fn insert_all(&mut self, atoms: &AtomTable, objects: Vec<Vec<Term>>) -> Vec<usize> {
        if self.table_type != TableType::Bag {
            let count = objects.len();
            for object in objects {
                self.insert(atoms, object);
            }
            return vec![0; count];
        }
        let mut walks: HashMap<u64, Vec<BagWalk>> = HashMap::new();
        objects
            .into_iter()
            .map(|object| self.insert_bag(atoms, object, &mut walks))
            .collect()
    }

    /// Add the object `object` of a `bag` table and return the number of
    /// objects that its insert compared, as the steps of one list insert
    /// give it ([`Table::insert_all`]). The walk of each key stays in
    /// `walks` across the objects of the list, so one list insert does the
    /// work of ERTS and not that work for each object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
    /// let atoms = AtomTable::new();
    /// let mut walks = HashMap::new();
    /// let object = vec![Term::Integer(1), Term::Integer(2)];
    /// assert_eq!(table.insert_bag(&atoms, object.clone(), &mut walks), 0);
    /// assert_eq!(table.insert_bag(&atoms, object, &mut walks), 1);
    /// ```
    fn insert_bag(
        &mut self,
        atoms: &AtomTable,
        object: Vec<Term>,
        walks: &mut HashMap<u64, Vec<BagWalk>>,
    ) -> usize {
        let hash = stored_hash(atoms, self.key(&object));
        let walks = walks.entry(hash).or_default();
        let index = match (walks.iter()).position(|walk| walk.key == *self.key(&object)) {
            Some(index) => index,
            None => {
                let key = self.key(&object).clone();
                let cursor = self.position(hash, &key).unwrap_or(EMPTY);
                walks.push(BagWalk::new(key, cursor));
                walks.len() - 1
            }
        };
        let walk = &mut walks[index];
        let object_hash = object_hash(&object);
        match walk.find(self, object_hash, &object) {
            Some(count) => count,
            None => {
                let count = walk.added + walk.walked;
                let newest = self.position(hash, &walk.key).unwrap_or(EMPTY);
                let slot = self.add(atoms, hash, object, newest);
                walk.seen
                    .entry(object_hash)
                    .or_default()
                    .push((Place::Added(walk.added), slot));
                walk.added += 1;
                count
            }
        }
    }

    /// Put `object` into a new slot as the newest object of its key, whose
    /// stored key hash is `hash`, before the object in the slot `newest`, or
    /// as the only object of its key when `newest` is `EMPTY`. Return the
    /// slot. A new key goes to the end of its bucket chain, and a new object
    /// of a key takes the place of the newest object of the key in the chain
    /// (`erl_db_hash.c:1384-1393`, `1447-1452`). A new object that makes the
    /// objects more than the buckets splits one bucket
    /// (`erl_db_hash.c:1453-1461`). `atoms` owns the name of each atom key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// let key = Term::Integer(1);
    /// assert_eq!(table.add(&atoms, stored_hash(&atoms, &key), vec![key], EMPTY), 0);
    /// ```
    fn add(&mut self, atoms: &AtomTable, hash: u64, object: Vec<Term>, newest: usize) -> usize {
        let slot = self.allocate();
        self.objects[slot] = Some(object);
        self.older[slot] = newest;
        self.writes += 1;
        let bucket = self.bucket(hash);
        if newest == EMPTY {
            self.index.insert(hash, slot);
            let last = self.chain_keys(bucket).last().unwrap_or(EMPTY);
            self.link(bucket, last, slot);
        } else {
            self.index.replace(hash, newest, slot);
            let before = self.key_before(bucket, newest);
            self.next[slot] = std::mem::replace(&mut self.next[newest], EMPTY);
            self.link(bucket, before, slot);
        }
        if self.object_count() > self.buckets {
            self.split(atoms);
        }
        slot
    }

    /// Move the key of the object in the slot `slot` from the stored hash
    /// `old` to the stored hash `new` in the key index and in the bucket
    /// chain. An `ordered_set` insert can replace the key of an object by an
    /// arithmetically equal key with another hash, as `1` and `1.0`
    /// (`erl_db_tree.c:758-761`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// table.rekey(0, stored_hash(&atoms, &Term::static_atom("k")), stored_hash(&atoms, &Term::static_atom("k")));
    /// ```
    fn rekey(&mut self, slot: usize, old: u64, new: u64) {
        if old == new {
            return;
        }
        self.index.remove(old, slot);
        self.index.insert(new, slot);
        let old_bucket = self.bucket(old);
        let after = self.next[slot];
        self.link(old_bucket, self.key_before(old_bucket, slot), after);
        let new_bucket = self.bucket(new);
        self.next[slot] = self.heads[new_bucket];
        self.link(new_bucket, EMPTY, slot);
    }

    /// Return the bucket of the stored key hash `hash`: the hash under the
    /// size mask, or under the half mask when that bucket is not active
    /// (`hash_to_ix`, `erl_db_hash.c:199-209`). The size mask is one less
    /// than the power of two that holds the buckets
    /// (`erl_db_hash.c:3675-3681`, `3776-3779`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// assert_eq!(table.bucket(0x1ff), 0xff);
    /// ```
    fn bucket(&self, hash: u64) -> usize {
        let mask = self.buckets.next_power_of_two() - 1;
        // `HashValue` is a `UWord`, so the key hash fits.
        let bucket = hash as usize & mask;
        if bucket >= self.buckets {
            bucket & (mask >> 1)
        } else {
            bucket
        }
    }

    /// Return the slot of the newest object of each key in the chain of the
    /// bucket `bucket`, in chain order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// assert_eq!(table.chain_keys(0).count(), 0);
    /// ```
    fn chain_keys(&self, bucket: usize) -> impl Iterator<Item = usize> + '_ {
        links(self.heads[bucket], &self.next)
    }

    /// Return the slot of the key before the key whose newest object is in
    /// the slot `slot` in the chain of the bucket `bucket`, or `EMPTY` when
    /// it is the first key. The caller makes sure that the chain holds it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1)]);
    /// assert_eq!(table.key_before(table.bucket(stored_hash(&atoms, &Term::Integer(1))), 0), EMPTY);
    /// ```
    fn key_before(&self, bucket: usize, slot: usize) -> usize {
        self.chain_keys(bucket)
            .take_while(|&key| key != slot)
            .last()
            .unwrap_or(EMPTY)
    }

    /// Make the key slot `slot` follow the key slot `before` in the chain of
    /// the bucket `bucket`, or make it the first key when `before` is
    /// `EMPTY`. The caller sets the link of `slot` itself.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.link(0, EMPTY, 3);
    /// assert_eq!(table.heads[0], 3);
    /// ```
    fn link(&mut self, bucket: usize, before: usize, slot: usize) {
        if before == EMPTY {
            self.heads[bucket] = slot;
        } else {
            self.next[before] = slot;
        }
    }

    /// Split one bucket into the new top bucket, as `grow`
    /// (`erl_db_hash.c:3648-3740`) does: each key of the low bucket whose
    /// hash under the new size mask is the new bucket goes to the new
    /// bucket, and both chains keep their order. `atoms` owns the name of
    /// each atom key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.split(&atoms);
    /// assert_eq!(table.buckets, 257);
    /// ```
    fn split(&mut self, atoms: &AtomTable) {
        let to = self.buckets;
        self.buckets += 1;
        self.heads.push(EMPTY);
        let mask = self.buckets.next_power_of_two() - 1;
        let from = to & (mask >> 1);
        let keys: Vec<usize> = self.chain_keys(from).collect();
        let (mut low, mut high) = (EMPTY, EMPTY);
        self.heads[from] = EMPTY;
        for slot in keys {
            self.next[slot] = EMPTY;
            let hash = stored_hash(atoms, self.key(self.object_of(slot)));
            if hash as usize & mask == to {
                self.link(to, high, slot);
                high = slot;
            } else {
                self.link(from, low, slot);
                low = slot;
            }
        }
    }

    /// Join the top bucket into its low bucket, as `shrink`
    /// (`erl_db_hash.c:3743-3818`) does: the chain of the top bucket goes
    /// before the chain of the low bucket (`erl_db_hash.c:3795-3808`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// table.join();
    /// assert_eq!(table.buckets, 4351);
    /// ```
    fn join(&mut self) {
        let source = self.buckets - 1;
        let target = source & (self.buckets.next_power_of_two() - 1) >> 1;
        self.buckets = source;
        let chain = self.heads.pop().expect("an active bucket has a head");
        if chain != EMPTY {
            let low = std::mem::replace(&mut self.heads[target], chain);
            let last = self
                .chain_keys(target)
                .last()
                .expect("the top chain has a key");
            self.next[last] = low;
        }
    }

    /// Return the elements of the newest object with a key that is exactly
    /// equal to `key`, or `None` when the table has no such object. A `set`
    /// table has at most one such object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// assert_eq!(table.object(&atoms, &Term::static_atom("k")), None);
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.object(&atoms, &Term::static_atom("k")), Some(&[Term::static_atom("k")][..]));
    /// ```
    pub(super) fn object(&self, atoms: &AtomTable, key: &Term) -> Option<&[Term]> {
        let slot = self.key_slot(atoms, key)?;
        Some(self.object_of(slot).as_slice())
    }

    /// Return the elements of each object with a key that is exactly equal
    /// to `key`, in insertion order, as `get_term_list` and `build_term_list`
    /// (`erl_db_hash.c:1469-1490`) give them: the walk reads the chain from
    /// the newest object and puts each object before the earlier ones.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::DuplicateBag);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k"), Term::Integer(1)]);
    /// table.insert(&atoms, vec![Term::static_atom("k"), Term::Integer(1)]);
    /// assert_eq!(table.objects(&atoms, &Term::static_atom("k")).len(), 2);
    /// assert!(table.objects(&atoms, &Term::static_atom("j")).is_empty());
    /// ```
    pub(super) fn objects(&self, atoms: &AtomTable, key: &Term) -> Vec<&[Term]> {
        let mut objects = Vec::new();
        let mut slot = self.key_slot(atoms, key).unwrap_or(EMPTY);
        while slot != EMPTY {
            objects.push(self.object_of(slot).as_slice());
            slot = self.older[slot];
        }
        objects.reverse();
        objects
    }

    /// Return the number of active buckets, as `NACTIVE` (`erl_db_hash.c:179`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1, TableType::Set).bucket_count(), 256);
    /// ```
    pub(super) fn bucket_count(&self) -> usize {
        self.buckets
    }

    /// Return the elements of each object in the chain of the bucket
    /// `bucket`, in chain order: the keys in chain order, and the objects of
    /// each key from the newest to the oldest, as a traversal reads them
    /// (`match_traverse`, `erl_db_hash.c:1897-1918`). The caller makes sure
    /// that the bucket is active.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Bag);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1), Term::Integer(1)]);
    /// table.insert(&atoms, vec![Term::Integer(1), Term::Integer(2)]);
    /// let bucket = table.key_bucket(&atoms, &Term::Integer(1)).unwrap();
    /// assert_eq!(table.chain(bucket).map(|object| object[1].clone()).collect::<Vec<_>>(), [Term::Integer(2), Term::Integer(1)]);
    /// ```
    pub(super) fn chain(&self, bucket: usize) -> impl Iterator<Item = &[Term]> + '_ {
        self.chain_keys(bucket).flat_map(|newest| {
            links(newest, &self.older).map(|slot| self.object_of(slot).as_slice())
        })
    }

    /// Return the bucket of `key` when the table holds an object with a key
    /// that is exactly equal to it, else `None`, as `analyze_pattern`
    /// (`erl_db_hash.c:3262-3276`) finds the bucket of a bound key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// assert_eq!(table.key_bucket(&AtomTable::new(), &Term::Integer(1)), None);
    /// ```
    pub(super) fn key_bucket(&self, atoms: &AtomTable, key: &Term) -> Option<usize> {
        let hash = stored_hash(atoms, key);
        self.position(hash, key).map(|_| self.bucket(hash))
    }

    /// Return the number of changes to the objects of the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// table.insert(&AtomTable::new(), vec![Term::Integer(1)]);
    /// assert_eq!(table.writes(), 1);
    /// ```
    pub(super) fn writes(&self) -> u64 {
        self.writes
    }

    /// Return the bytes of the container arrays of the table: the slot array
    /// of the objects, the free-slot array, and the element array of each
    /// stored object, the array of the older slot of each slot, the head of
    /// each bucket chain, and the next key of each slot. The number excludes the header of the `Vec` of each
    /// array, as the accounting of task
    /// [#559](https://github.com/fahchen/pbeam/issues/559) excludes it. An
    /// object of immediate elements owns its element array, so the number is
    /// above 0 for it.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1), Term::Integer(2)]);
    /// assert!(table.allocated_bytes() > 0);
    /// ```
    pub(super) fn allocated_bytes(&self) -> usize {
        self.objects.capacity() * std::mem::size_of::<Option<Vec<Term>>>()
            + (self.free.capacity()
                + self.older.capacity()
                + self.heads.capacity()
                + self.next.capacity())
                * std::mem::size_of::<usize>()
            + self
                .objects
                .iter()
                .flatten()
                .map(|object| object.capacity() * std::mem::size_of::<Term>())
                .sum::<usize>()
            + self.busy.as_ref().map_or(0, |busy| {
                busy.objects.capacity() * std::mem::size_of::<Vec<Term>>()
                    + busy
                        .objects
                        .iter()
                        .map(|object| object.capacity() * std::mem::size_of::<Term>())
                        .sum::<usize>()
                    + busy.walks.capacity() * std::mem::size_of::<(u64, Vec<BagWalk>)>()
                    + busy
                        .walks
                        .values()
                        .map(|walks| {
                            walks.capacity() * std::mem::size_of::<BagWalk>()
                                + walks
                                    .iter()
                                    .map(|walk| {
                                        walk.seen
                                            .values()
                                            .map(|slots| {
                                                slots.capacity()
                                                    * std::mem::size_of::<(Place, usize)>()
                                            })
                                            .sum::<usize>()
                                    })
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            })
    }

    /// Return the slot of the object whose stored key hash is `hash` and whose
    /// key is exactly equal to `key`, or `None` when the table has no such
    /// object. The walk reads the index slot of the hash (`hash_to_ix`,
    /// `erl_db_hash.c:207-218`), compares the stored hash value, and compares
    /// the stored key when the two hashes are equal (`has_live_key`,
    /// `erl_db_hash.c:766-776`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.position(stored_hash(&Term::static_atom("k")), &Term::static_atom("k")), Some(0));
    /// ```
    /// Return the slot of the object whose key is `key`. A `set` table finds
    /// the slot by the stored hash of an exactly equal key. An `ordered_set`
    /// table compares keys with the arithmetic term order, as the ERTS tree
    /// does: `cmp_key_eq` tests the exact equality first and then compares
    /// with `CMP` (`erl_db_tree.c:758-761`), so `1` finds the object with
    /// the key `1.0`. The exact lookup runs first, because an exactly equal
    /// key has the same stored hash. A walk of the slots then compares each
    /// key with `Term::compare`. A comparison error continues the walk: the
    /// order of two different references is the only error
    /// (`src/term/order.rs`), and a term with another reference is not equal.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::OrderedSet);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1), Term::static_atom("a")]);
    /// assert_eq!(table.key_slot(&atoms, &Term::Float(1.0)), Some(0));
    /// assert_eq!(table.key_slot(&atoms, &Term::Integer(2)), None);
    /// ```
    fn key_slot(&self, atoms: &AtomTable, key: &Term) -> Option<usize> {
        if let Some(slot) = self.position(stored_hash(atoms, key), key) {
            return Some(slot);
        }
        if self.table_type != TableType::OrderedSet {
            return None;
        }
        self.objects.iter().enumerate().find_map(|(slot, object)| {
            let object = object.as_ref()?;
            matches!(self.key(object).compare(key, atoms), Ok(Ordering::Equal)).then_some(slot)
        })
    }

    fn position(&self, hash: u64, key: &Term) -> Option<usize> {
        self.index
            .find(hash, |slot| self.key(self.object_of(slot)) == key)
    }

    /// Return the elements of the object in the slot `slot`. The caller makes
    /// sure that the slot is live.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.object_of(0), &vec![Term::static_atom("k")]);
    /// ```
    fn object_of(&self, slot: usize) -> &Vec<Term> {
        self.objects[slot]
            .as_ref()
            .expect("a live slot holds an object")
    }

    /// Return the number of live objects.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.object_count(), 1);
    /// ```
    fn object_count(&self) -> usize {
        self.objects.len() - self.free.len()
    }

    /// Return a slot for a new object: a free slot when the table has one,
    /// else a new slot at the end of the table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// assert_eq!(table.allocate(), 0);
    /// assert_eq!(table.allocate(), 1);
    /// ```
    fn allocate(&mut self) -> usize {
        match self.free.pop() {
            Some(slot) => slot,
            None => {
                self.objects.push(None);
                self.older.push(EMPTY);
                self.next.push(EMPTY);
                self.objects.len() - 1
            }
        }
    }

    /// Remove each object with a key that is exactly equal to `key`, as
    /// `db_erase_hash` (`erl_db_hash.c:1613-1664`) does. A key that the
    /// table does not hold changes nothing. The slot of each object becomes
    /// free, and a later insert reuses it. A removal joins the top bucket
    /// into its low bucket, up to 5 times, while the objects are fewer than
    /// the shrink limit (`try_shrink` and `shrink`, `erl_db_hash.c:757-762`,
    /// `3743-3818`). ERTS tries the shrink once after it removes all objects
    /// of the key (`erl_db_hash.c:1653-1660`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// table.delete(&atoms, &Term::static_atom("missing"));
    /// table.delete(&atoms, &Term::static_atom("k"));
    /// assert_eq!(table.object(&atoms, &Term::static_atom("k")), None);
    /// ```
    pub(super) fn delete(&mut self, atoms: &AtomTable, key: &Term) {
        let Some(newest) = self.key_slot(atoms, key) else {
            return;
        };
        let hash = stored_hash(atoms, self.key(self.object_of(newest)));
        self.index.remove(hash, newest);
        self.writes += 1;
        let bucket = self.bucket(hash);
        let before = self.key_before(bucket, newest);
        let after = std::mem::replace(&mut self.next[newest], EMPTY);
        self.link(bucket, before, after);
        let mut slot = newest;
        while slot != EMPTY {
            self.objects[slot] = None;
            self.free.push(slot);
            slot = std::mem::replace(&mut self.older[slot], EMPTY);
        }
        for _ in 0..RESIZE_STEPS {
            if self.object_count() >= self.shrink_limit() {
                break;
            }
            self.join();
        }
    }

    /// Return the number of slots of the ERTS hash table: the first segment
    /// and each later segment that has an active bucket. A split allocates a
    /// segment when the buckets fill the slots (`erl_db_hash.c:3667-3672`),
    /// and a join frees the top segment when it has no active bucket
    /// (`erl_db_hash.c:3783-3785`). The key index has its own slots and does
    /// not use this count.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1, TableType::Set).segment_slots(), 256);
    /// ```
    fn segment_slots(&self) -> usize {
        FIRST_SEGMENT_SLOTS
            + (self.buckets - FIRST_SEGMENT_SLOTS).div_ceil(SEGMENT_SLOTS) * SEGMENT_SLOTS
    }

    /// Return the object count below which a delete joins buckets: half the
    /// slots without the top segment, or 0 below 4352 slots
    /// (`calc_shrink_limit`, `erl_db_hash.c:3350-3407`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1, TableType::Set).shrink_limit(), 0);
    /// ```
    fn shrink_limit(&self) -> usize {
        let slots = self.segment_slots();
        if slots >= SHRINK_SLOTS {
            (slots - SEGMENT_SLOTS) / 2
        } else {
            0
        }
    }

    /// Return the key of `object`: its element at the key position.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 2, TableType::Set);
    /// assert_eq!(table.key(&[Term::static_atom("v"), Term::static_atom("k")]), &Term::static_atom("k"));
    /// ```
    fn key<'a>(&self, object: &'a [Term]) -> &'a Term {
        let index = usize::try_from(self.keypos).expect("a stored object has a key position") - 1;
        &object[index]
    }
}

#[cfg(test)]
impl Table {
    /// Return the bytes of the two vectors of the key index. The measurement
    /// harness of [#539](https://github.com/fahchen/pbeam/issues/539) reports
    /// it, so the space of the index is a measured number.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1, TableType::Set).index_bytes(), 0);
    /// ```
    pub(super) fn index_bytes(&self) -> usize {
        self.index.hashes.capacity() * std::mem::size_of::<u64>()
            + self.index.slots.capacity() * std::mem::size_of::<usize>()
    }

    /// Return the live objects in slot order, for a test.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.live_objects(), vec![vec![Term::static_atom("k")]]);
    /// ```
    pub(super) fn live_objects(&self) -> Vec<Vec<Term>> {
        self.objects.iter().flatten().cloned().collect()
    }

    /// Return the number of allocated slots, for a test.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::static_atom("k")]);
    /// assert_eq!(table.slot_count(), 1);
    /// ```
    pub(super) fn slot_count(&self) -> usize {
        self.objects.len()
    }
}

/// Where an object of a [`BagWalk`] is in the chain of its key.
#[derive(Debug, Clone, Copy)]
enum Place {
    /// The list insert added it as the object with this 0-based number.
    Added(usize),
    /// It was in the table, at this 0-based position from the newest object
    /// that was in the table.
    Walked(usize),
}

/// The state of one key in [`Table::insert_all`] of a `bag` table.
#[derive(Debug)]
struct BagWalk {
    /// The key.
    key: Term,
    /// The next slot of the objects that were in the table, or `EMPTY`
    /// when the walk read each of them.
    cursor: usize,
    /// The number of objects of the key that were in the table and that the
    /// walk read.
    walked: usize,
    /// The number of objects of the key that the list insert added.
    added: usize,
    /// Each added or walked object by the hash of the whole object
    /// ([`object_hash`]), with its place and its slot.
    seen: HashMap<u64, Vec<(Place, usize)>>,
}

impl BagWalk {
    /// Return the walk of the key `key` whose newest object in the table is
    /// in the slot `cursor`, or `EMPTY` for a new key.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let walk = BagWalk::new(Term::Integer(1), EMPTY);
    /// assert_eq!((walk.walked, walk.added), (0, 0));
    /// ```
    fn new(key: Term, cursor: usize) -> BagWalk {
        BagWalk {
            key,
            cursor,
            walked: 0,
            added: 0,
            seen: HashMap::new(),
        }
    }

    /// Return the number of objects that ERTS compares before it finds an
    /// object that is exactly equal to `object`, whose hash is `hash`, or
    /// `None` when the key has no such object. The chain of the key has the
    /// added objects first, the newest first, and then the objects that
    /// were in the table. An added object is never equal to another object
    /// of the chain, so the first equal object is the only one. The walk
    /// reads the objects that were in the table only up to the equal one.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut walk = BagWalk::new(Term::Integer(1), EMPTY);
    /// let object = vec![Term::Integer(1)];
    /// assert_eq!(walk.find(&table, object_hash(&object), &object), None);
    /// ```
    fn find(&mut self, table: &Table, hash: u64, object: &[Term]) -> Option<usize> {
        let place = |seen: &HashMap<u64, Vec<(Place, usize)>>| {
            seen.get(&hash)?
                .iter()
                .find(|(_, slot)| table.object_of(*slot) == object)
                .map(|(place, _)| *place)
        };
        let place = place(&self.seen).or_else(|| {
            while self.cursor != EMPTY {
                let slot = self.cursor;
                let walked = self.walked;
                self.cursor = table.older[slot];
                self.walked += 1;
                let candidate = table.object_of(slot);
                let candidate_hash = object_hash(candidate);
                self.seen
                    .entry(candidate_hash)
                    .or_default()
                    .push((Place::Walked(walked), slot));
                if candidate == object {
                    return Some(Place::Walked(walked));
                }
            }
            None
        })?;
        Some(match place {
            Place::Added(number) => self.added - 1 - number,
            Place::Walked(position) => self.added + position,
        })
    }
}

/// The key index of one `set` table: the stored hash of each key to the slot
/// of its object in [`Table::objects`].
///
/// The index is an open-addressing table with linear probing. A lookup reads
/// the slot that the ERTS size mask of the hash gives (`hash_to_ix`,
/// `erl_db_hash.c:207-218`), then compares the stored hash value of the entry
/// and the stored key only when the two hashes are equal (`has_live_key`,
/// `erl_db_hash.c:766-776`). The index stores one 64-bit hash and one slot id
/// for each object. A slot id is stable while the table is live, so a delete
/// holds no scan of the index.
#[derive(Debug, Default)]
struct KeyIndex {
    /// The stored key hash of each index slot, as the `hvalue` of an ERTS
    /// hash table entry (`HashDbTerm`), or 0 in a free slot.
    hashes: Vec<u64>,
    /// The object slot of each index slot, or `EMPTY` in a free slot.
    slots: Vec<usize>,
    /// The number of live entries.
    len: usize,
}

impl KeyIndex {
    /// Return the object slot of the entry whose stored hash is `hash` and
    /// whose key `matches`, or `None` when the index holds no such entry.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::default();
    /// index.insert(7, 0);
    /// assert_eq!(index.find(7, |_| true), Some(0));
    /// assert_eq!(index.find(8, |_| true), None);
    /// ```
    fn find(&self, hash: u64, matches: impl Fn(usize) -> bool) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        let mask = self.slots.len() - 1;
        let mut slot = hash as usize & mask;
        loop {
            let object = self.slots[slot];
            if object == EMPTY {
                return None;
            }
            if self.hashes[slot] == hash && matches(object) {
                return Some(object);
            }
            slot = (slot + 1) & mask;
        }
    }

    /// Add the entry with the stored hash `hash` and the object slot `object`.
    /// The caller made sure that the index holds no entry with that hash and a
    /// matching key. The slot count doubles when the live entries reach half
    /// of it (`grow`, `erl_db_hash.c:3648-3740`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::default();
    /// index.insert(7, 4);
    /// assert_eq!(index.find(7, |_| true), Some(4));
    /// ```
    fn insert(&mut self, hash: u64, object: usize) {
        if 2 * (self.len + 1) > self.slots.len() {
            self.grow();
        }
        let mask = self.slots.len() - 1;
        let mut slot = hash as usize & mask;
        while self.slots[slot] != EMPTY {
            slot = (slot + 1) & mask;
        }
        self.hashes[slot] = hash;
        self.slots[slot] = object;
        self.len += 1;
    }

    /// Double the slot count and add each live entry again. The entries keep
    /// their stored hashes and their object slots, so a growth hashes no key
    /// again and moves no object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::default();
    /// for hash in 0..5 {
    ///     index.insert(hash, hash as usize);
    /// }
    /// assert_eq!(index.slots.len(), 16);
    /// ```
    fn grow(&mut self) {
        let slots = if self.slots.is_empty() {
            INITIAL_INDEX_SLOTS
        } else {
            self.slots.len() * 2
        };
        let hashes = std::mem::replace(&mut self.hashes, vec![0; slots]);
        let objects = std::mem::replace(&mut self.slots, vec![EMPTY; slots]);
        let mask = slots - 1;
        self.len = 0;
        for (hash, object) in hashes.into_iter().zip(objects) {
            if object == EMPTY {
                continue;
            }
            let mut slot = hash as usize & mask;
            while self.slots[slot] != EMPTY {
                slot = (slot + 1) & mask;
            }
            self.hashes[slot] = hash;
            self.slots[slot] = object;
            self.len += 1;
        }
    }

    /// Set the object slot of the entry with the stored hash `hash` and the
    /// object slot `old` to `new`. A new object of a bag key becomes the
    /// head of the chain of its key, as `*bp = q` does in `db_put_hash`
    /// (`erl_db_hash.c:1451-1452`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::default();
    /// index.insert(7, 0);
    /// index.replace(7, 0, 3);
    /// assert_eq!(index.find(7, |_| true), Some(3));
    /// ```
    fn replace(&mut self, hash: u64, old: usize, new: usize) {
        let mask = self.slots.len() - 1;
        let mut entry = hash as usize & mask;
        while self.slots[entry] != old {
            entry = (entry + 1) & mask;
        }
        self.slots[entry] = new;
    }

    /// Remove the entry with the stored hash `hash` and the object slot
    /// `object`. The hole of the entry closes by a backward shift: an entry
    /// moves into the hole when its home slot is not between the hole and the
    /// entry in the circular probe order. OTP unlinks a chained entry with a
    /// pointer write (`b = *bp = next`, `erl_db_hash.c:1641`) and performs no
    /// shift; the shift is the choice of this open-addressing index, and it
    /// keeps the rule that an entry stays reachable from its home slot. The
    /// object slot of each other entry does not change, so a delete walks no
    /// other entry.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut index = KeyIndex::default();
    /// index.insert(7, 0);
    /// index.insert(9, 1);
    /// index.remove(7, 0);
    /// assert_eq!(index.find(7, |_| true), None);
    /// assert_eq!(index.find(9, |_| true), Some(1));
    /// ```
    fn remove(&mut self, hash: u64, object: usize) {
        let mask = self.slots.len() - 1;
        let mut entry = hash as usize & mask;
        while self.slots[entry] != object {
            entry = (entry + 1) & mask;
        }
        let mut hole = entry;
        let mut next = entry;
        loop {
            next = (next + 1) & mask;
            if self.slots[next] == EMPTY {
                break;
            }
            let home = self.hashes[next] as usize & mask;
            let blocked = if hole <= next {
                hole < home && home <= next
            } else {
                hole < home || home <= next
            };
            if !blocked {
                self.hashes[hole] = self.hashes[next];
                self.slots[hole] = self.slots[next];
                hole = next;
            }
        }
        self.slots[hole] = EMPTY;
        self.len -= 1;
    }
}

/// Return a hash of the whole object `object`: the standard SipHash
/// ([`DefaultHasher`]) of the ERTS internal hash of each element
/// ([`internal_hash`]), in element order. Two exactly equal objects have
/// equal hashes, so [`BagWalk`] compares only the objects with the hash of
/// the object. SipHash mixes each element hash into the full state, so two
/// elements that change together do not cancel, as they did with a rotate
/// and XOR of the element hashes.
///
/// # Examples
///
/// ```rust,ignore
/// let object = [Term::Integer(1), Term::Integer(2)];
/// assert_eq!(object_hash(&object), object_hash(&object.clone()));
/// ```
fn object_hash(object: &[Term]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for term in object {
        hasher.write_u64(internal_hash(term));
    }
    hasher.finish()
}

/// Return the stored hash of an ETS key: the ERTS key hash ([`key_hash`])
/// shifted one bit right, as `MAKE_HASH` gives the `hvalue` of an ERTS hash
/// table entry (`erl_db_hash.c:279-282`). `atoms` owns the name of an atom
/// key.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// assert_eq!(stored_hash(&atoms, &Term::Integer(1)), key_hash(&atoms, &Term::Integer(1)) >> 1);
/// ```
fn stored_hash(atoms: &AtomTable, key: &Term) -> u64 {
    key_hash(atoms, key) >> 1
}

/// Return the slots of a linked chain from the slot `first`, where `links`
/// holds the next slot of each slot and `EMPTY` ends the chain.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(links(0, &[2, EMPTY, 1]).collect::<Vec<_>>(), [0, 2, 1]);
/// assert_eq!(links(EMPTY, &[]).count(), 0);
/// ```
fn links(first: usize, links: &[usize]) -> impl Iterator<Item = usize> + '_ {
    std::iter::successors((first != EMPTY).then_some(first), |&slot| {
        Some(links[slot]).filter(|&next| next != EMPTY)
    })
}

/// Return the segment of the slot `slot`: the first segment has the slots
/// below 256, and each later segment the next 2048 slots
/// (`SLOT_IX_TO_SEG_IX`, `erl_db_hash.c:191`).
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(segment(255), 0);
/// assert_eq!(segment(256), 1);
/// assert_eq!(segment(2304), 2);
/// ```
fn segment(slot: usize) -> usize {
    (slot + SEGMENT_SLOTS - FIRST_SEGMENT_SLOTS) / SEGMENT_SLOTS
}

/// Return the hash of an ETS key before the shift (`MAKE_HASH`,
/// `erl_db_hash.c:279-282`): the atom table hash of an atom, else the ERTS
/// internal hash. `atoms` owns the name of an atom key.
///
/// # Examples
///
/// ```rust,ignore
/// let atoms = AtomTable::new();
/// assert_eq!(key_hash(&atoms, &Term::static_atom("a")), 0x61);
/// assert_eq!(key_hash(&atoms, &Term::Integer(1)), 7938660269479274665);
/// ```
fn key_hash(atoms: &AtomTable, key: &Term) -> u64 {
    match key {
        Term::Atom(atom) => atom_hash(atoms, atom),
        _ => internal_hash(key),
    }
}

#[cfg(test)]
#[path = "../../tests/runtime/ets.rs"]
mod tests;
