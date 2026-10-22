//! ETS tables of the node, as the ERTS table store (`erl_db.c`). All
//! processes of a run share one store.

pub(super) mod pattern;

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::hash::{DefaultHasher, Hasher};
use std::sync::{Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

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

/// The ETS tables of the node behind the lock of the table store, which
/// each scheduler of a run shares, as ERTS keeps one meta table for the
/// node (`meta_name_tab`, `erl_db.c:420-433`). A BIF that reaches one table takes the shared
/// lock and then the lock of the table ([`EtsTables::read`],
/// [`EtsTables::write`]). A change of the store itself, such as a create, a
/// delete, a fix, the exit of an owner, or a clear step, takes the
/// exclusive lock, which excludes each holder of a table lock too.
#[derive(Debug, Default)]
pub(super) struct NodeTables(RwLock<EtsTables>);

impl NodeTables {
    /// Return the table store under its shared lock. A thread takes this
    /// lock at most once at a time, so a waiting exclusive lock cannot
    /// block it.
    pub(super) fn shared(&self) -> RwLockReadGuard<'_, EtsTables> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Return the table store under its exclusive lock. See
    /// [`NodeTables::shared`].
    pub(super) fn exclusive(&self) -> RwLockWriteGuard<'_, EtsTables> {
        self.0.write().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The ETS tables of the node. ERTS identifies each table by a table
/// reference that `make_btid` (`erl_db.c:2811`) gives it, also a named
/// table, and finds a named table by its atom in the meta name table. An
/// atom name is unique in one atom table, so its text identifies it, as in
/// `Scheduler::names`. PBeam supports `set`, `bag`, and `duplicate_bag`
/// tables.
#[derive(Debug, Default)]
pub(super) struct EtsTables {
    /// Each live table by its table reference, each with its own lock, as
    /// each ERTS table has its rwlock (`erl_db_util.h:322`). The ETS BIFs
    /// reach a table only through [`EtsTables::read`],
    /// [`EtsTables::write`], [`EtsTables::find`], and [`EtsTables::busy`],
    /// which take that lock, as `db_get_table` and `db_lock` do
    /// (`erl_db.c:701-921`).
    tables: HashMap<Reference, RwLock<Table>>,
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
    /// The fixations of each process by their insertion number, in the
    /// order that it fixed the tables, as the fixed-table list of the
    /// process (`erl_db.c:130-150`). One entry for each table: a second fix
    /// of the same table adds no entry (`fix_table_locked`,
    /// `erl_db.c:5150-5192`).
    fixed: HashMap<ProcessId, BTreeMap<u64, Reference>>,
    /// The insertion number of each fixation, so an unfix or a delete
    /// removes its entry without a scan (`fixed_tabs_delete`,
    /// `erl_db.c:152-172`).
    fixed_seq: HashMap<(ProcessId, Reference), u64>,
    /// The last insertion number that `fixed` holds.
    next_fix: u64,
    /// The tables whose deletion leaves the order entry of a process that
    /// did not delete them, so the process removes the entry and pays one
    /// reduction when it runs (`erts_schedule_ets_free_fixation`,
    /// `erl_db.c:5263-5301`).
    pending: HashMap<ProcessId, VecDeque<Reference>>,
    /// The processes that became pending since the last scheduling, so the
    /// caller enqueues only them (`erts_schedule_ets_free_fixation`,
    /// `erl_db.c:5263-5301`).
    pending_new: Vec<ProcessId>,
    /// The compiled match specification of each `ets:select/1`
    /// continuation, by the reference that the continuation holds in place
    /// of the ERTS magic reference (`ets_db_make_match_prog_ref`,
    /// `erl_db_hash.c:2103`, `2229`). Each entry also holds the table
    /// reference, so the delete of the table drops its programs.
    programs: HashMap<Reference, (Reference, pattern::MatchSpec)>,
    /// The result of each list insert that another process completed, by
    /// the process that started it, which reads it in its next turn
    /// (`erl_db.c:2142-2150`, `2232-2244`). The starter can run on another
    /// scheduler, so the result waits here.
    /// A process gets an entry when its insert takes the table
    /// ([`EtsTables::reserve_result`]), and its exit removes the entry, so
    /// a helper that ends the insert of a dead process keeps no result, as
    /// the continuation of ERTS goes with its process.
    results: Mutex<HashMap<ProcessId, Option<Term>>>,
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
    /// The number of delete records that are not free yet, which the free
    /// consumes before the segments (`db_free_table_continue_hash`,
    /// `erl_db_hash.c:3122-3137`).
    fixdels: usize,
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
    /// The clear of a fixed table that did not finish, as the trap state of
    /// its `FixedDeletion` (`erl_db_hash.c:3007-3051`).
    clear: Option<Clearing>,
    /// The processes that fixed the table with `ets:safe_fixtable(Tab,
    /// true)`, one entry for each fix. While one exists, the table keeps
    /// each deleted object as a pseudo-deleted object and does not split or
    /// join a bucket, as ERTS keeps a fixated table (`erl_db_hash.c:49-60`).
    fixed_by: HashMap<ProcessId, usize>,
    /// The keys that a process deleted while the table was fixed, with the
    /// slots of their objects. The objects stay in the table, and their
    /// chain nodes stay in the bucket chains, until the last unfix.
    retained: HashMap<u64, Vec<Retained>>,
    /// The stored hashes of the retained records by bucket, so a segment
    /// free visits only the buckets that have a record
    /// (`db_free_table_continue_hash`, `erl_db_hash.c:3122-3149`).
    retained_buckets: BTreeMap<usize, Vec<u64>>,
    /// The difference between the ERTS item counter and the live object
    /// count. Only the take of a fixed key makes it nonzero, because the
    /// counter falls by each record of the key and the kept pseudo-deleted
    /// objects count as items (`db_take_hash`, `erl_db_hash.c:2949-2995`).
    size_adjust: i64,
    /// The number of delete records that the fixation keeps, as the count of
    /// the ERTS `FixedDeletion` records (`erl_db_hash.c:221-272`): one for a
    /// delete of a key, one for a clear, and one for each bucket of a select
    /// delete.
    fixdels: usize,
    /// The bucket of the last object that the current select delete removed,
    /// as `select_delete_context_t::last_pseudo_delete`
    /// (`erl_db_hash.c:2611`), or `EMPTY` before its first removal.
    select_slot: usize,
    /// For each slot, whether the slot holds a pseudo-deleted object that a
    /// fixation keeps.
    deleted: Vec<bool>,
    /// The number of pseudo-deleted objects, which `object_count` leaves
    /// out.
    deleted_objects: usize,
}

/// A key that a process deleted while the table was fixed: its stored
/// hash, the key, the slot of its chain node, and each slot that held an
/// object of the key. The chain node stays in its bucket chain until the
/// last unfix, as the pseudo-deleted object of a fixed ERTS table does
/// (`erl_db_hash.c:49-60`).
#[derive(Debug)]
struct Retained {
    key: Term,
    newest: usize,
    slots: Vec<usize>,
}

/// A clear of a fixed table that did not finish in its turn, as the
/// `FixedDeletion` with `trap` set (`db_mark_all_deleted_hash`,
/// `erl_db_hash.c:3007-3051`): the process that started it, the count that
/// the clear returns, and the bucket where the mark continues.
#[derive(Debug)]
pub(super) struct Clearing {
    owner: ProcessId,
    count: usize,
    bucket: usize,
    /// The number of segments that the clear has not freed yet after the
    /// last unfix ended the fixation, or `None` while the clear marks
    /// buckets (`db_free_table_continue_hash`, `erl_db_hash.c:3140-3149`).
    free: Option<usize>,
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

/// Return the bucket of the stored key hash `hash` for a table of `buckets`
/// buckets, as `hash_to_ix` gives it (`erl_db_hash.c:199-209`).
fn stored_bucket(hash: u64, buckets: usize) -> usize {
    let mask = buckets.next_power_of_two() - 1;
    // `HashValue` is a `UWord`, so the key hash fits.
    let bucket = hash as usize & mask;
    if bucket >= buckets {
        bucket & (mask >> 1)
    } else {
        bucket
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

/// `ITERATIONS_PER_RED` (`erl_db.c:2152`): the units of one reduction of
/// the budget of a list insert.
pub(super) const ITERATIONS_PER_RED: i64 = 8;

/// Why a process cannot use a table now, as `handle_lacking_permission`
/// (`erl_db.c:769-798`) returns no table.
#[derive(Debug, Clone, Copy)]
pub(super) enum Lacking {
    /// No live table, or no permission: the BIF raises `badarg`
    /// (`erl_db.c:788-795`).
    Denied,
    /// The list insert of another process or a clear held the table busy.
    /// The caller helped with the work, which used `reductions`, and the
    /// BIF traps to run again (`erl_db.c:773-787`).
    Busy {
        /// The reductions of the help.
        reductions: i64,
    },
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
    /// `DB_INFO`, which each table allows (`erl_db_util.h:451`).
    Info,
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
        self.tables.insert(id, RwLock::new(table));
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
        let locked: Vec<_> = self.tables.values().map(read).collect();
        let freeing = self.freeing.values().map(|freeing| &freeing.table);
        for table in locked.iter().map(|table| &**table).chain(freeing) {
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

    /// Return the table that `id` identifies under its read lock, when the
    /// process `caller` can read it, as `db_get_table` with `DB_READ`
    /// (`erl_db.c:800-912`) and `db_lock` with `LCK_READ` (`erl_db.c:701`)
    /// do. An atom finds a named table, and a reference finds its table.
    /// Return [`Lacking::Denied`] for each other term, a free name, a deleted
    /// table, and a process that is not the owner of a `private` table
    /// (`handle_lacking_permission`, `erl_db.c:788-795`). The ETS BIFs then
    /// raise `badarg`. A table that the list insert of another process or a
    /// clear holds busy gets the help of the caller under the table lock,
    /// and gives [`Lacking::Busy`] (`erl_db.c:773-787`). The BIF then charges
    /// the reductions and traps to run again.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Protected, 1, TableType::Set));
    /// let name = Term::static_atom("ac_tab");
    /// assert!(tables.read(&atoms, &name, ProcessId(1), 4000).is_ok());
    /// assert!(tables.read(&atoms, &Term::Integer(1), ProcessId(0), 4000).is_err());
    /// ```
    pub(super) fn read(
        &self,
        atoms: &AtomTable,
        id: &Term,
        caller: ProcessId,
        reds: i64,
    ) -> Result<RwLockReadGuard<'_, Table>, Lacking> {
        self.take_read(atoms, id, caller, reds, Operation::Read)
    }

    /// Return the table that `id` identifies under its read lock, as
    /// [`EtsTables::read`] does for `DB_INFO` (`erl_db_util.h:451`), which
    /// each process has.
    pub(super) fn info(
        &self,
        atoms: &AtomTable,
        id: &Term,
        caller: ProcessId,
        reds: i64,
    ) -> Result<RwLockReadGuard<'_, Table>, Lacking> {
        self.take_read(atoms, id, caller, reds, Operation::Info)
    }

    /// Take the read lock of a table for the operation `operation`. See
    /// [`EtsTables::read`].
    fn take_read(
        &self,
        atoms: &AtomTable,
        id: &Term,
        caller: ProcessId,
        reds: i64,
        operation: Operation,
    ) -> Result<RwLockReadGuard<'_, Table>, Lacking> {
        let (_, lock) = self.lookup(atoms, id).ok_or(Lacking::Denied)?;
        loop {
            let table = read(lock);
            if !table.lacks(caller) {
                return table
                    .allows(caller, operation)
                    .then_some(table)
                    .ok_or(Lacking::Denied);
            }
            drop(table);
            // `handle_lacking_permission` takes the write lock for the help
            // (`erl_db.c:773-777`). The table can change between the two
            // locks, so the check runs again.
            if let Some(reductions) = self.help(atoms, &mut write(lock), caller, reds) {
                return Err(Lacking::Busy { reductions });
            }
        }
    }

    /// Run the work of the list insert of another process or of the clear
    /// that holds the table `table` busy, with the budget `reds` of the
    /// caller, as `handle_lacking_permission` (`erl_db.c:773-787`) runs
    /// `ets_insert_2_list_continuation` or `delete_all_objects_continue`
    /// under the table lock. Return the reductions that the work used, as
    /// `erl_db.c:2152-2197` and `3300-3320` charge them, or `None` when the
    /// table is not busy for the caller. A list insert keeps the result of
    /// its end for the process that started it, before the lock goes, as
    /// ERTS sets `return_value` before it clears `DB_BUSY`
    /// (`erl_db.c:2183-2192`).
    fn help(
        &self,
        atoms: &AtomTable,
        table: &mut Table,
        caller: ProcessId,
        reds: i64,
    ) -> Option<i64> {
        if let Some(owner) = (table.busy.as_ref()).map(|busy| busy.owner) {
            if owner == caller {
                return None;
            }
            // `ets_insert_2_list_continuation` keeps `ITERATIONS_PER_RED *
            // ERTS_BIF_REDS_LEFT` in a `long`, and YCF yields when the count
            // is 0 or less (`erl_db.c:2158`, `ycf_yield_fun.c:592-610`), so
            // a negative budget does what a budget of 0 does.
            let (used, result) = table.step_busy(atoms, ITERATIONS_PER_RED * reds.max(0));
            if let Some(value) = result {
                self.put_result(owner, value);
            }
            return Some(used / ITERATIONS_PER_RED);
        }
        let (owner, count) = (table.clear.as_ref()).map(|clear| (clear.owner, clear.count))?;
        // A process that uses the table while a fixed clear does not finish
        // helps with the mark, 8 buckets for each reduction, with a signed
        // budget (`erl_db.c:3300-3320`).
        let (_, charge) = table.step_clear(atoms, owner, count, reds);
        Some(i64::try_from(charge).unwrap_or(i64::MAX))
    }

    /// Return the table that `id` identifies under its write lock, when the
    /// process `caller` can write it, as `db_get_table` with `DB_WRITE`
    /// (`erl_db.c:800-912`) and `db_lock` with `LCK_WRITE` (`erl_db.c:701`)
    /// do. Return the outcomes of [`EtsTables::read`], and also
    /// [`Lacking::Denied`] for a `protected` table of another owner.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut tables = EtsTables::default();
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Protected, 1, TableType::Set));
    /// let name = Term::static_atom("ac_tab");
    /// assert!(tables.write(&atoms, &name, ProcessId(1), 4000).is_err());
    /// assert!(tables.write(&atoms, &name, ProcessId(0), 4000).is_ok());
    /// ```
    pub(super) fn write(
        &self,
        atoms: &AtomTable,
        id: &Term,
        caller: ProcessId,
        reds: i64,
    ) -> Result<RwLockWriteGuard<'_, Table>, Lacking> {
        let (_, lock) = self.lookup(atoms, id).ok_or(Lacking::Denied)?;
        let mut table = write(lock);
        if let Some(reductions) = self.help(atoms, &mut table, caller, reds) {
            return Err(Lacking::Busy { reductions });
        }
        table
            .allows(caller, Operation::Write)
            .then_some(table)
            .ok_or(Lacking::Denied)
    }

    /// Return the table reference of the live table that `id` identifies,
    /// with no lock and no access check, as the meta table of ERTS gives it
    /// (`erl_db.c:800-912`). Return `None` as [`EtsTables::find`] does.
    pub(super) fn id(&self, atoms: &AtomTable, id: &Term) -> Option<Reference> {
        self.lookup(atoms, id).map(|(id, _)| id)
    }

    /// Return the reference and the lock of the live table that `id`
    /// identifies: an atom finds a named table in the name table, and a
    /// reference finds its table, as `db_get_table_aux`
    /// (`erl_db.c:800-912`) does. Return `None` for each other term, a free
    /// name, and a deleted table.
    fn lookup(&self, atoms: &AtomTable, id: &Term) -> Option<(Reference, &RwLock<Table>)> {
        let id = match id {
            Term::Atom(name) => *self.names.get(atoms.name_of(name))?,
            Term::Reference(id) => *id,
            _ => return None,
        };
        self.tables.get(&id).map(|table| (id, table))
    }

    /// Return the live table `id` with no lock, for the work of the table
    /// store that has the store to itself (`&mut self`).
    fn live(&mut self, id: Reference) -> Option<&mut Table> {
        let table = self.tables.get_mut(&id)?;
        Some(table.get_mut().unwrap_or_else(PoisonError::into_inner))
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
    pub(super) fn find(
        &self,
        atoms: &AtomTable,
        id: &Term,
    ) -> Option<(Reference, RwLockReadGuard<'_, Table>)> {
        let (id, table) = self.lookup(atoms, id)?;
        Some((id, read(table)))
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
    pub(super) fn busy(&self, atoms: &AtomTable, id: &Term) -> Option<RwLockWriteGuard<'_, Table>> {
        let table = write(self.lookup(atoms, id)?.1);
        table.busy.is_some().then_some(table)
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
        self.live(id)
    }

    /// Keep the result `value` of the list insert that the process
    /// `process` started, for its next turn.
    pub(super) fn put_result(&self, process: ProcessId, value: Term) {
        let mut results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = results.get_mut(&process) {
            *slot = Some(value);
        }
    }

    /// Take the result of the list insert of the process `process` that
    /// another process completed.
    pub(super) fn take_result(&self, process: ProcessId) -> Option<Term> {
        let mut results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        results.get(&process)?.as_ref()?;
        results.remove(&process).flatten()
    }

    /// Keep an empty result entry for the process `process`, whose list
    /// insert takes a table, so a later [`EtsTables::put_result`] keeps its
    /// result until the process reads it or exits.
    pub(super) fn reserve_result(&self, process: ProcessId) {
        let mut results = self.results.lock().unwrap_or_else(PoisonError::into_inner);
        results.insert(process, None);
    }

    /// Fix the table `id` for the process `process`, and add the fixation
    /// to the order of the fixations of the process (`fixed_tabs_insert`,
    /// `erl_db.c:135-150`).
    pub(super) fn fix_table(&mut self, id: Reference, process: ProcessId) {
        let first = !(self.live(id))
            .expect("a fixed table is live")
            .fixed_by
            .contains_key(&process);
        if first {
            self.next_fix += 1;
            let seq = self.next_fix;
            self.fixed.entry(process).or_default().insert(seq, id);
            self.fixed_seq.insert((process, id), seq);
        }
        self.live(id).expect("a fixed table is live").fix(process);
    }

    /// Unfix the table `id` for the process `process`, and drop the order
    /// entry of the last fixation of the process (`fixed_tabs_delete`,
    /// `erl_db.c:152-172`).
    pub(super) fn unfix_table(&mut self, id: Reference, process: ProcessId) {
        let table = self.live(id).expect("an unfixed table is live");
        table.unfix(process);
        if !table.fixed_by.contains_key(&process) {
            // A process that did not fix the table has no entry here, and a
            // second unfix of the last fixation changes nothing
            // (`unfix_table_locked`, `erl_db.c:1153-1157`).
            self.forget_fixation(id, process);
        }
    }

    /// Remove the order entry of the fixation of the process `process` on
    /// the table `id`, and the list of the process when it is empty
    /// (`fixed_tabs_delete`, `erl_db.c:152-172`).
    fn forget_fixation(&mut self, id: Reference, process: ProcessId) {
        let Some(seq) = self.fixed_seq.remove(&(process, id)) else {
            return;
        };
        let empty = {
            let list = self
                .fixed
                .get_mut(&process)
                .expect("a fixing process has a fixation list");
            list.remove(&seq);
            list.is_empty()
        };
        if empty {
            self.fixed.remove(&process);
        }
    }

    /// Remove each fixation of the table `id` from the list of the process
    /// `process`, as `free_fixations_locked` does (`erl_db.c:5304-5318`).
    /// The caller gives the processes that fixed the table, so the delete
    /// reads no other list.
    fn forget_fixations(&mut self, id: Reference, fixers: &[ProcessId], caller: ProcessId) {
        for process in fixers {
            if *process == caller {
                self.forget_fixation(id, *process);
            } else {
                // The entry of another process stays until that process runs
                // and then removes it, or until its exit
                // (`erts_schedule_ets_free_fixation`, `erl_db.c:5263-5301`).
                self.pending.entry(*process).or_default().push_back(id);
                self.pending_new.push(*process);
            }
        }
    }

    /// Remove each fixation entry of the pending cleanups of the process
    /// `process` and return their count, so the caller charges one
    /// reduction for each (`proc_cleanup_fixed_table`,
    /// `erl_db.c:4975-5015`). Return 0 when the process has no pending
    /// cleanup.
    pub(super) fn drain_pending(&mut self, process: ProcessId, limit: usize) -> usize {
        let (drained, empty) = {
            let Some(ids) = self.pending.get_mut(&process) else {
                return 0;
            };
            let mut drained = Vec::with_capacity(limit.min(ids.len()));
            while drained.len() < limit {
                let Some(id) = ids.pop_front() else {
                    break;
                };
                drained.push(id);
            }
            (drained, ids.is_empty())
        };
        if empty {
            self.pending.remove(&process);
        }
        for id in &drained {
            self.forget_fixation(*id, process);
        }
        drained.len()
    }

    /// Return the processes that became pending since the last call, and
    /// forget them.
    pub(super) fn take_pending_new(&mut self) -> Vec<ProcessId> {
        std::mem::take(&mut self.pending_new)
    }

    /// Return the number of pending cleanups of the process `process`.
    pub(super) fn pending_count(&self, process: ProcessId) -> usize {
        (self.pending.get(&process)).map_or(0, VecDeque::len)
    }

    /// Delete the tables that the process `owner` owns, as
    /// `erts_db_process_exiting` (`erl_db.c:5031-5144`) does when the owner
    /// exits, with `reds` reductions left in the turn. Return the reductions
    /// that are left when each table is free and each fixation of the
    /// process is gone, or `None` when the exit must switch out and call
    /// again in its next turn.
    ///
    /// The tables go in the order of the owned list of `owner`, from its
    /// head (`erl_db.c:5055-5096`): see `Table::owned`. A table that a
    /// trapped `ets:delete/1` of `owner` frees keeps its position in the
    /// list, so an older table goes first. A live table first loses its name and its reference,
    /// so no other process finds it, and an insert that traps on it then
    /// raises `badarg` (`erl_db.c:5082-5094`). The free of each table
    /// charges one reduction for each process that fixed it
    /// (`free_fixations_locked`, `erl_db.c:5304-5318`). Then each segment,
    /// from the top segment down, uses 32 reductions and 1 for each object
    /// in it (`erl_db_hash.c:3122-3149`, `3468-3530`). When the reductions
    /// are less than 0 after a segment, or 0 after a table, the exit
    /// switches out (`erl_db.c:5098-5104`, `5131`).
    ///
    /// After the owned tables, the exit removes the fixations of `owner` on
    /// the remaining tables (`erl_db.c:5110-5126`). Each fixation uses one
    /// reduction, and the last fixation of a table also uses one for each
    /// pseudo-deleted object and each delete record that it frees
    /// (`proc_cleanup_fixed_table`, `erl_db.c:4975-5015`). The exit
    /// switches out when no reduction is left.
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
            let live = (self.tables.iter_mut()).map(|(id, table)| {
                let table = table.get_mut().unwrap_or_else(PoisonError::into_inner);
                (*id, &*table)
            });
            let freeing = (self.freeing.iter()).map(|(id, freeing)| (*id, &freeing.table));
            let Some((_, id)) = live
                .chain(freeing)
                .filter(|(_, table)| table.owner == owner)
                .map(|(id, table)| (table.owned, id))
                .min()
            else {
                break;
            };
            if let Some(table) = self.tables.remove(&id) {
                let mut table = table.into_inner().unwrap_or_else(PoisonError::into_inner);
                let fixers = table.fixers();
                reds -= i32::try_from(fixers.len()).unwrap_or(i32::MAX);
                // The table goes, so each fixation of it goes too
                // (`free_fixations_locked`, `erl_db.c:5304-5318`).
                table.fixed_by.clear();
                self.forget_fixations(id, &fixers, owner);
                self.names.retain(|_, named| *named != id);
                self.programs.retain(|_, (table, _)| *table != id);
                self.freeing.insert(id, Freeing::new(table));
            }
            reds = self.free_table(id, reds)?;
            if reds == 0 {
                return None;
            }
        }
        // The exit now removes the fixations of the process on the tables
        // that stay, in the order of its fixed-table list
        // (`erl_db.c:5110-5126`).
        loop {
            let Some((seq, id)) = (self.fixed.get(&owner))
                .and_then(|fixed| fixed.first_key_value())
                .map(|(&seq, &id)| (seq, id))
            else {
                self.fixed.remove(&owner);
                break;
            };
            match self.live(id) {
                Some(table) => {
                    reds -= i32::try_from(table.unfix_cost(owner)).unwrap_or(i32::MAX);
                    table.unfix_all(owner);
                }
                // The cleanup of a fixation on a deleted table charges one
                // reduction (`proc_cleanup_fixed_table`, `erl_db.c:4975-5015`).
                None => reds -= 1,
            }
            // The exit frees each fixation of the process on the table, so
            // the fixed-table list of the process drops the table too
            // (`proc_cleanup_fixed_table`, `erl_db.c:4975-5015`).
            let empty = {
                let fixed = self
                    .fixed
                    .get_mut(&owner)
                    .expect("the process has a fixation");
                fixed.remove(&seq);
                fixed.is_empty()
            };
            self.fixed_seq.remove(&(owner, id));
            if empty {
                self.fixed.remove(&owner);
            }
            if reds <= 0 {
                return None;
            }
        }
        self.pending.remove(&owner);
        (self.results.get_mut())
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&owner);
        Some(reds)
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
    pub(super) fn delete_table(&mut self, id: Reference, caller: ProcessId) -> usize {
        let mut table = (self.tables.remove(&id))
            .expect("a deleted table is live")
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner);
        self.names.retain(|_, named| *named != id);
        self.programs.retain(|_, (table, _)| *table != id);
        if table.owner != caller {
            table.owner = caller;
            table.owned = self.save_owned();
        }
        // The delete frees each fixation of the table, with one reduction for
        // each process that fixed it (`free_fixations_locked`,
        // `erl_db.c:3056`).
        let fixers = table.fixers();
        table.fixed_by.clear();
        self.forget_fixations(id, &fixers, caller);
        // The free of the table frees each delete record with the segments
        // (`db_free_table_continue_hash`, `erl_db_hash.c:3122-3137`).
        let charge = fixers.len();
        self.freeing.insert(id, Freeing::new(table));
        charge
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

/// Take the read lock of the table `table`. A panic under the lock leaves
/// no half change that a reader can see, as a BIF panic stops the run.
fn read(table: &RwLock<Table>) -> RwLockReadGuard<'_, Table> {
    table.read().unwrap_or_else(PoisonError::into_inner)
}

/// Take the write lock of the table `table`. See [`read`].
fn write(table: &RwLock<Table>) -> RwLockWriteGuard<'_, Table> {
    table.write().unwrap_or_else(PoisonError::into_inner)
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
    fn new(mut table: Table) -> Freeing {
        let segments = if table.table_type == TableType::OrderedSet {
            1
        } else {
            segment(table.segment_slots() - 1) + 1
        };
        let fixdels = table.fixdels;
        table.fixdels = 0;
        Freeing {
            table,
            segments,
            bucket: 0,
            next: EMPTY,
            fixdels,
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
        // The delete records free before the segments, one reduction each
        // (`db_free_table_continue_hash`, `erl_db_hash.c:3122-3137`).
        if self.fixdels > 0 {
            self.fixdels -= 1;
            return Some(1);
        }
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
        table.forget_retained_segment(first, last);
        Some(reds)
    }

    /// Free the objects of an `ordered_set` table, one more than the
    /// reductions `budget` of the turn, as `do_free_tree_continue` frees a
    /// node for each reduction and returns when the reductions are less
    /// than 0 (`erl_db_tree.c:2877-2904`). Return the number of objects that
    /// are free, or `None` when the table holds no object. The walk follows
    /// the bucket chains of the live keys, and [`Table::compact`] keeps the
    /// slots dense, so the free of the table drops one slot for each object
    /// that is in it and no slot of an object that an earlier delete freed,
    /// as the ERTS walk reads only the live nodes of the tree.
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
    /// Return whether the process `caller` can do `operation` on the table:
    /// the owner can do each operation, and the access mode gives the
    /// rights of each other process (`erl_db.c:877-883`).
    fn allows(&self, caller: ProcessId, operation: Operation) -> bool {
        self.owner == caller
            || matches!(
                (self.access, operation),
                (Access::Public, _) | (Access::Protected, Operation::Read) | (_, Operation::Info)
            )
    }

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
            clear: None,
            fixed_by: HashMap::new(),
            retained: HashMap::new(),
            retained_buckets: BTreeMap::new(),
            size_adjust: 0,
            fixdels: 0,
            select_slot: EMPTY,
            deleted: Vec::new(),
            deleted_objects: 0,
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
    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.object_count()
    }

    /// Return the ERTS item counter, which `ets:info(Table, size)` gives: the
    /// live objects and the adjustment of a fixed take, wrapped as the
    /// unsigned `Uint` counter (`ADD_NITEMS`, `erl_db_hash.c:2949-2995`).
    pub(super) fn size(&self) -> u64 {
        let structural = i128::try_from(self.object_count()).unwrap_or(i128::MAX);
        let size = structural + i128::from(self.size_adjust);
        (size as u128 & u128::from(u64::MAX)) as u64
    }

    /// Return the item counter as the signed `Sint` that growth and shrink
    /// compare (`NITEMS_ESTIMATE`, `erl_db_hash.c:1352`, `759`).
    fn item_count(&self) -> i64 {
        self.size() as i64
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
        if self.is_fixed() {
            // A fixed table marks each bucket, 8 buckets for each reduction,
            // and the loop does not count the last bucket
            // (`db_mark_all_deleted_hash`, `erl_db_hash.c:3034-3051`).
            return (self.buckets - 1).div_ceil(8);
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
        if self.position(hash, key).is_some() {
            let node = self
                .chain_node(hash, key)
                .expect("a live key has a chain node");
            return Some(self.next_live_after(self.bucket(hash), self.next[node]));
        }
        // A fixed table keeps each deleted key as a pseudo-deleted object,
        // so a traversal that reads the deleted key continues after it
        // (`erl_db_hash.c:49-60`).
        let retained = self
            .retained
            .get(&hash)?
            .iter()
            .rev()
            .find(|retained| retained.key == *key)?;
        Some(self.next_live_after(self.bucket(hash), self.next[retained.newest]))
    }

    /// Return the first key in the order of a hash table, as
    /// `db_first_hash_common` (`erl_db_hash.c:1117-1140`) gives it: the key
    /// of the first live object from bucket 0 up. Return `None` for an empty
    /// table.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// assert_eq!(table.first_key(), None);
    /// ```
    pub(super) fn first_key(&self) -> Option<Term> {
        self.next_live_after(0, self.heads[0])
    }

    /// Return the key of the first slot that is not deleted after the slot
    /// `slot` of the bucket `bucket`, or `None` at the end of the table.
    fn next_live_after(&self, mut bucket: usize, mut slot: usize) -> Option<Term> {
        loop {
            while slot != EMPTY {
                if !self.is_deleted(slot) || self.object_chain_has_live(slot) {
                    return Some(self.key(self.object_of(slot)).clone());
                }
                slot = self.next[slot];
            }
            bucket += 1;
            if bucket == self.buckets {
                return None;
            }
            slot = self.heads[bucket];
        }
    }

    /// Start a select-delete traversal, whose first removal adds a delete
    /// record for its bucket (`erl_db_hash.c:2611`, `2636-2640`).
    pub(super) fn begin_select_delete(&mut self) {
        self.select_slot = EMPTY;
    }

    /// Return whether the slot holds a pseudo-deleted object.
    fn is_deleted(&self, slot: usize) -> bool {
        self.deleted.get(slot).copied().unwrap_or(false)
    }

    /// Return whether the key of the chain node `node` has a live object.
    fn object_chain_has_live(&self, node: usize) -> bool {
        links(node, &self.older).any(|slot| !self.is_deleted(slot))
    }

    /// Return the slot of the chain node of the key with the stored hash
    /// `hash`: the slot that the bucket chain of the key links, whose
    /// object is the first object of the key, also when a fixation kept
    /// that object as a pseudo-deleted object.
    fn chain_node(&self, hash: u64, key: &Term) -> Option<usize> {
        let bucket = self.bucket(hash);
        self.chain_keys(bucket)
            .find(|&slot| self.key(self.object_of(slot)) == key)
    }

    /// Mark the slot `slot` as a pseudo-deleted object.
    fn mark_deleted(&mut self, slot: usize) {
        if self.deleted.len() <= slot {
            self.deleted.resize(slot + 1, false);
        }
        self.deleted[slot] = true;
    }

    /// Keep the objects of the key `key` as pseudo-deleted objects, as the
    /// delete of a fixed ERTS table does (`erl_db_hash.c:1631-1645`): remove
    /// the key from the index, keep each pseudo-deleted object, mark the
    /// first live object pseudo-deleted, free each other live object, and
    /// keep the chain node of the key in its bucket chain. The delete adds
    /// one delete record (`add_fixed_deletion`, `erl_db_hash.c:263-269`).
    fn retain_key(&mut self, hash: u64, key: &Term, node: usize) {
        if let Some(index_slot) = self.position(hash, key) {
            self.index.remove(hash, index_slot);
        }
        let mut kept = Vec::new();
        let mut slot = node;
        let mut kept_live = false;
        while slot != EMPTY {
            let older = self.older[slot];
            if self.is_deleted(slot) {
                kept.push(slot);
            } else if !kept_live {
                self.mark_deleted(slot);
                self.deleted_objects += 1;
                kept.push(slot);
                kept_live = true;
            } else {
                self.objects[slot] = None;
                self.older[slot] = EMPTY;
                self.free.push(slot);
            }
            slot = older;
        }
        for pair in kept.windows(2) {
            self.older[pair[0]] = pair[1];
        }
        if let Some(&last) = kept.last() {
            self.older[last] = EMPTY;
        }
        self.fixdels += 1;
        self.retain_record(hash, key, node);
    }

    /// Set the retained record of the key with the stored hash `hash` to the
    /// object chain from the chain node `node`, in the order of the objects,
    /// or add the record. The record of an earlier delete takes the new
    /// order.
    fn retain_record(&mut self, hash: u64, key: &Term, node: usize) {
        let mut slots = Vec::new();
        let mut slot = node;
        while slot != EMPTY {
            slots.push(slot);
            slot = self.older[slot];
        }
        let records = self.retained.entry(hash).or_default();
        if let Some(record) = records.iter_mut().rev().find(|record| record.key == *key) {
            record.slots = slots;
            record.newest = node;
        } else {
            records.push(Retained {
                key: key.clone(),
                newest: node,
                slots,
            });
            let bucket = stored_bucket(hash, self.buckets);
            self.retained_buckets.entry(bucket).or_default().push(hash);
        }
    }

    /// Return the newest live object of the key after the slot `slot` of
    /// the object chain, or `None` when the key has no other live object.
    fn next_live_object(&self, slot: usize) -> Option<usize> {
        let mut object = self.older[slot];
        while object != EMPTY {
            if !self.is_deleted(object) {
                return Some(object);
            }
            object = self.older[object];
        }
        None
    }

    /// Restore the pseudo-deleted object in the slot `slot` as the newest
    /// object of its key, as the insert of an object that a fixed table
    /// keeps does (`erl_db_hash.c:1426-1437`): the object becomes live and
    /// moves to the head of the object chain.
    fn restore_object(&mut self, slot: usize, hash: u64, key: &Term) {
        self.deleted[slot] = false;
        self.deleted_objects -= 1;
        let node = self
            .chain_node(hash, key)
            .expect("a deleted key has a chain node");
        if slot != node {
            let mut previous = node;
            while self.older[previous] != slot {
                previous = self.older[previous];
            }
            self.older[previous] = self.older[slot];
            let bucket = self.bucket(hash);
            let before = self.key_before(bucket, node);
            self.next[slot] = std::mem::replace(&mut self.next[node], EMPTY);
            self.older[slot] = node;
            self.link(bucket, before, slot);
        }
        match self.position(hash, key) {
            Some(current) => self.index.replace(hash, current, slot),
            None => self.index.insert(hash, slot),
        }
        self.retain_record(hash, key, slot);
        self.writes += 1;
    }

    /// Free each pseudo-deleted object and remove it from its object chain,
    /// as the last unfix of an ERTS table does (`erl_db_hash.c:49-60`). A
    /// key whose chain node is pseudo-deleted but that still has a live
    /// object takes its first live object as the chain node, and a key with
    /// no live object leaves the bucket chain.
    fn drop_retained(&mut self) {
        if self.retained.is_empty() {
            // A reinserted key removed its retained entry, and the unfix
            // frees the delete records that went with it
            // (`db_unfix_table_hash`, `erl_db_hash.c:939-990`).
            self.fixdels = 0;
            self.deleted_objects = 0;
            return;
        }
        let mut buckets = (self.retained.keys())
            .map(|hash| self.bucket(*hash))
            .collect::<Vec<_>>();
        buckets.sort_unstable();
        buckets.dedup();
        for bucket in buckets {
            let mut node = self.heads[bucket];
            let mut head = EMPTY;
            let mut previous = EMPTY;
            while node != EMPTY {
                let next = self.next[node];
                let mut chain = vec![node];
                let mut object = self.older[node];
                while object != EMPTY {
                    chain.push(object);
                    object = self.older[object];
                }
                let mut live = Vec::new();
                for object in chain {
                    if self.is_deleted(object) {
                        self.objects[object] = None;
                        self.older[object] = EMPTY;
                        self.next[object] = EMPTY;
                        self.free.push(object);
                    } else {
                        live.push(object);
                    }
                }
                if let Some(&first) = live.first() {
                    let mut older = EMPTY;
                    for &object in live.iter().rev() {
                        self.older[object] = older;
                        older = object;
                    }
                    if previous == EMPTY {
                        head = first;
                    } else {
                        self.next[previous] = first;
                    }
                    previous = first;
                }
                node = next;
            }
            if previous != EMPTY {
                self.next[previous] = EMPTY;
            }
            self.heads[bucket] = head;
        }
        self.retained.clear();
        self.retained_buckets.clear();
        self.fixdels = 0;
        self.deleted.clear();
        self.deleted_objects = 0;
    }

    /// Fix the table for the process `process`, as `fix_table_locked`
    /// (`erl_db.c:1150-1152`) does. A second fix by one process changes
    /// nothing.
    pub(super) fn fix(&mut self, process: ProcessId) {
        // Each fix counts in one entry of the process, and the matching
        // unfix decrements it (`erl_db.c:5158-5169`).
        *self.fixed_by.entry(process).or_insert(0) += 1;
    }

    /// Unfix the table for the process `process`, as `unfix_table_locked`
    /// (`erl_db.c:1153-1157`) does. The last unfix drops the keys that the
    /// fixation kept.
    pub(super) fn unfix(&mut self, process: ProcessId) {
        if let Some(count) = self.fixed_by.get_mut(&process) {
            *count -= 1;
            if *count == 0 {
                self.fixed_by.remove(&process);
            }
        }
        if self.fixed_by.is_empty() {
            self.drop_retained();
        }
    }

    /// Remove each fixation of the process `process`, as the exit of the
    /// process does (`db_fixation_cleanup`, `erl_db.c:5207-5211`).
    pub(super) fn unfix_all(&mut self, process: ProcessId) {
        self.fixed_by.remove(&process);
        if self.fixed_by.is_empty() {
            self.drop_retained();
        }
    }
    /// holds the table busy.
    /// Return whether the list insert of another process, or a clear with
    /// no list insert, holds the table busy for `caller` (`DB_BUSY`,
    /// `erl_db.c:773`).
    fn lacks(&self, caller: ProcessId) -> bool {
        match &self.busy {
            Some(busy) => busy.owner != caller,
            None => self.clear.is_some(),
        }
    }

    /// Return whether a process fixed the table.
    pub(super) fn is_fixed(&self) -> bool {
        !self.fixed_by.is_empty()
    }

    /// Return each process that fixed the table once, as the entries that
    /// `free_fixations_locked` frees (`erl_db.c:5304-5318`). A second fix
    /// of one process counts once, because ERTS keeps one entry with a
    /// count.
    fn fixers(&self) -> Vec<ProcessId> {
        self.fixed_by.keys().copied().collect()
    }

    /// Return the reductions of the exit cleanup of the fixations of
    /// `process` on the table, as `proc_cleanup_fixed_table` charges them
    /// (`erl_db.c:4975-5015`): one for the fixation, and, when the table is
    /// not fixed again after the cleanup, one for each pseudo-deleted object
    /// and one for each delete record that the unfix frees
    /// (`db_unfix_table_hash`, `erl_db_hash.c:939-990`).
    fn unfix_cost(&self, process: ProcessId) -> usize {
        let last = self.fixed_by.keys().all(|fixed| *fixed == process);
        let work = if last {
            self.deleted_objects + self.fixdels
        } else {
            0
        };
        1 + work
    }

    /// Mark each object of the bucket `bucket` as a pseudo-deleted object of
    /// its key, as the loop of `db_mark_all_deleted_hash` does
    /// (`erl_db_hash.c:3034-3048`), and remove the key from the index.
    fn mark_bucket(&mut self, atoms: &AtomTable, bucket: usize) {
        for slot in self.chain_keys(bucket).collect::<Vec<_>>() {
            let key = self.key(self.object_of(slot)).clone();
            let hash = stored_hash(atoms, &key);
            if let Some(index_slot) = self.position(hash, &key) {
                self.index.remove(hash, index_slot);
            }
            let mut object = slot;
            while object != EMPTY {
                if !self.is_deleted(object) {
                    self.mark_deleted(object);
                    self.deleted_objects += 1;
                }
                object = self.older[object];
            }
            self.retain_record(hash, &key, slot);
        }
    }

    /// Mark the next buckets of a fixed table as pseudo-deleted, as the trap
    /// loop of `db_mark_all_deleted_hash` does (`erl_db_hash.c:3007-3051`):
    /// one bucket for each unit of `units`, and at least one bucket, with one
    /// delete record for the clear. Return `true` when each bucket is
    /// marked, and keep the state of an unfinished clear for the next turn.
    ///
    /// `reds` is the signed budget of the caller, an `SWord` in ERTS
    /// (`erl_db.c:3236`, `3303`). A negative budget marks one bucket, as a
    /// budget of 0 does (`erl_db_hash.c:3012`, `3033-3037`). The free of
    /// a table that is not fixed keeps a negative budget, so a free with
    /// nothing left to free does not finish (`erl_db_hash.c:3140-3165`,
    /// `4029-4031`).
    pub(super) fn step_clear(
        &mut self,
        atoms: &AtomTable,
        owner: ProcessId,
        count: usize,
        reds: i64,
    ) -> (bool, usize) {
        let mut clearing = self.clear.take().unwrap_or(Clearing {
            owner,
            count,
            bucket: 0,
            free: None,
        });
        if !self.is_fixed() {
            // The last unfix ended the fixation, so the continuation frees
            // the table as `db_delete_all_objects_hash` does without a
            // fixation (`erl_db_hash.c:4017-4037`). The free stops after the
            // segment that uses the last reduction
            // (`db_free_table_continue_hash`, `erl_db_hash.c:3140-3149`).
            if clearing.free.is_none() {
                clearing.free = Some(segment(self.segment_slots() - 1) + 1);
            }
            let mut remaining = reds;
            let mut charge = 0;
            while let Some(left) = clearing.free {
                if left == 0 {
                    // Each segment is free, and the next turn creates the
                    // table (`db_create_hash`, `erl_db_hash.c:4036-4037`).
                    break;
                }
                // The loop of `db_free_table_continue_hash` stops after the
                // segment that makes the budget negative
                // (`erl_db_hash.c:3140-3149`).
                if charge > 0 && remaining < 0 {
                    break;
                }
                let index = left - 1;
                let cost = self.segment_free_cost(index);
                self.free_segments_objects(index);
                charge += cost;
                remaining -= i64::try_from(cost).unwrap_or(i64::MAX);
                clearing.free = Some(index);
            }
            if clearing.free != Some(0) || remaining < 0 {
                self.clear = Some(clearing);
                return (false, charge);
            }
            self.drop_retained();
            self.reset_cleared();
            self.clear = None;
            return (true, charge);
        }
        if clearing.bucket == 0 {
            self.fixdels += 1;
            self.writes += 1;
        }
        let start = clearing.bucket;
        let limit = clearing
            .bucket
            .saturating_add(usize::try_from(reds).unwrap_or(0).saturating_mul(8).max(1));
        while clearing.bucket < self.buckets && clearing.bucket < limit {
            let bucket = clearing.bucket;
            self.mark_bucket(atoms, bucket);
            clearing.bucket += 1;
        }
        let done = clearing.bucket == self.buckets;
        let processed = clearing.bucket - start;
        if done {
            // The mark ends with `RESET_NITEMS`
            // (`db_mark_all_deleted_hash`, `erl_db_hash.c:3048-3050`).
            self.size_adjust = 0;
        } else {
            self.clear = Some(clearing);
        }
        // The loop of `db_mark_all_deleted_hash` does not count its last
        // bucket (`erl_db_hash.c:3034-3051`).
        let counted = if done {
            processed.saturating_sub(1)
        } else {
            processed
        };
        (done, counted.div_ceil(8))
    }

    /// Free each object of the segment `index` and remove each of its bucket
    /// chains, so the objects of one segment go in the turn that frees it
    /// (`db_free_table_continue_hash`, `erl_db_hash.c:3140-3149`).
    fn free_segments_objects(&mut self, index: usize) {
        let first = match index {
            0 => 0,
            later => FIRST_SEGMENT_SLOTS + (later - 1) * SEGMENT_SLOTS,
        };
        let last = (FIRST_SEGMENT_SLOTS + index * SEGMENT_SLOTS).min(self.buckets);
        for bucket in first..last {
            let mut node = std::mem::replace(&mut self.heads[bucket], EMPTY);
            while node != EMPTY {
                let mut slot = node;
                node = self.next[node];
                while slot != EMPTY {
                    let older = self.older[slot];
                    self.objects[slot] = None;
                    self.older[slot] = EMPTY;
                    self.next[slot] = EMPTY;
                    self.free.push(slot);
                    slot = older;
                }
            }
        }
        self.forget_retained_segment(first, last);
    }

    /// Return the reductions of the free of the segment `index`, as
    /// `db_free_table_continue_hash` charges them (`erl_db_hash.c:3141`):
    /// 32 for the segment and one for each object in it.
    fn segment_free_cost(&self, index: usize) -> usize {
        let first = match index {
            0 => 0,
            later => FIRST_SEGMENT_SLOTS + (later - 1) * SEGMENT_SLOTS,
        };
        let last = (FIRST_SEGMENT_SLOTS + index * SEGMENT_SLOTS).min(self.buckets);
        let mut objects = 0;
        for bucket in first..last {
            for node in self.chain_keys(bucket) {
                let mut slot = node;
                while slot != EMPTY {
                    objects += 1;
                    slot = self.older[slot];
                }
            }
        }
        SEGMENT_REDUCTIONS as usize + objects
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
    /// assert_eq!(table.clear(&atoms), 1);
    /// assert_eq!(table.len(), 0);
    /// ```
    pub(super) fn clear(&mut self, atoms: &AtomTable) -> usize {
        let count = self.object_count();
        if self.is_fixed() && self.table_type != TableType::OrderedSet {
            // A fixed table marks each object of each bucket as a
            // pseudo-deleted object and keeps its chains, with one delete
            // record (`db_mark_all_deleted_hash`, `erl_db_hash.c:3007-3051`).
            for bucket in 0..self.buckets {
                self.mark_bucket(atoms, bucket);
            }
            self.size_adjust = 0;
            self.fixdels += 1;
            self.writes += 1;
            return count;
        }
        self.reset_cleared();
        count
    }

    /// Empty the table for a clear that keeps no object, as `db_create_hash`
    /// does for a table without a fixation (`erl_db_hash.c:4036-4037`).
    fn reset_cleared(&mut self) {
        self.objects.clear();
        self.free.clear();
        self.older.clear();
        self.index = KeyIndex::default();
        self.heads = vec![EMPTY; FIRST_SEGMENT_SLOTS];
        self.next.clear();
        self.buckets = FIRST_SEGMENT_SLOTS;
        self.size_adjust = 0;
        self.writes += 1;
    }

    /// Take the objects of the key, as `db_take_hash` does
    /// (`erl_db_hash.c:2949-2995`): a fixed table keeps the first live object
    /// of the key as a pseudo-deleted object and frees each record after it,
    /// also the pseudo-deleted ones. The ERTS item counter falls by the
    /// number of those records, so the table size counts them and can go
    /// below 0. A table without a fixation removes the objects as a delete
    /// does.
    pub(super) fn take_key(&mut self, atoms: &AtomTable, key: &Term) {
        let hash = stored_hash(atoms, key);
        let Some(index_slot) = self.position(hash, key) else {
            return;
        };
        if !self.is_fixed() || self.table_type == TableType::OrderedSet {
            self.delete(atoms, key);
            return;
        }
        let node = self
            .chain_node(hash, key)
            .expect("a live key has a chain node");
        self.index.remove(hash, index_slot);
        let mut kept = Vec::new();
        let mut slot = node;
        let mut seen_live = false;
        let mut live = 0i64;
        let mut records = 0i64;
        while slot != EMPTY {
            let older = self.older[slot];
            if !seen_live && self.is_deleted(slot) {
                kept.push(slot);
            } else if !seen_live {
                seen_live = true;
                live += 1;
                records += 1;
                self.mark_deleted(slot);
                self.deleted_objects += 1;
                kept.push(slot);
            } else {
                records += 1;
                if self.is_deleted(slot) {
                    self.deleted_objects -= 1;
                } else {
                    live += 1;
                }
                self.objects[slot] = None;
                self.older[slot] = EMPTY;
                self.free.push(slot);
            }
            slot = older;
        }
        for pair in kept.windows(2) {
            self.older[pair[0]] = pair[1];
        }
        if let Some(&last) = kept.last() {
            self.older[last] = EMPTY;
        }
        self.size_adjust += live - records;
        self.fixdels += 1;
        self.retain_record(hash, key, node);
        self.writes += 1;
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
        let Some(index_slot) = self.position(hash, &key) else {
            return false;
        };
        let mut previous = EMPTY;
        let mut slot = index_slot;
        while slot != EMPTY {
            if !self.is_deleted(slot) && self.object_of(slot).as_slice() == object {
                if self.is_fixed() && self.table_type != TableType::OrderedSet {
                    // The object stays as a pseudo-deleted object of its key
                    // (`erl_db_hash.c:2634-2648`).
                    self.mark_deleted(slot);
                    self.deleted_objects += 1;
                    // The select delete adds one delete record for each
                    // bucket that it changes (`erl_db_hash.c:2636-2640`).
                    let bucket = self.bucket(hash);
                    if self.select_slot != bucket {
                        self.fixdels += 1;
                        self.select_slot = bucket;
                    }
                    let node = self
                        .chain_node(hash, &key)
                        .expect("a live key has a chain node");
                    // A select delete marks objects without a change of the
                    // chain, so the first removal registers the chain and
                    // each later one keeps it (`erl_db_hash.c:2622-2653`).
                    let registered = self.retained.get(&hash).is_some_and(|records| {
                        records
                            .iter()
                            .rev()
                            .any(|record| record.key == key && record.newest == node)
                    });
                    if !registered {
                        self.retain_record(hash, &key, node);
                    }
                    if slot == index_slot {
                        match self.next_live_object(slot) {
                            Some(next_live) => self.index.replace(hash, slot, next_live),
                            None => self.index.remove(hash, slot),
                        }
                    }
                    self.writes += 1;
                    return true;
                }
                self.delete_live_object(hash, slot, previous);
                return true;
            }
            previous = slot;
            slot = self.older[slot];
        }
        false
    }

    /// Remove the live object in the slot `slot` of the object chain whose
    /// slot before it is `previous`, as an unfixed table does: unlink it,
    /// move the chain node when the removed object was the node, and free
    /// the slot (`erl_db_hash.c:2622-2653`).
    fn delete_live_object(&mut self, hash: u64, slot: usize, previous: usize) {
        let bucket = self.bucket(hash);
        let next = self.older[slot];
        if previous == EMPTY {
            let after = self.next[slot];
            if next == EMPTY {
                self.index.remove(hash, slot);
                self.link(bucket, self.key_before(bucket, slot), after);
            } else {
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
    }

    /// Join buckets after a `select_delete` traversal, as
    /// `select_delete_on_loop_ended` calls `try_shrink` once
    /// (`erl_db_hash.c:2677-2709`).
    pub(super) fn shrink(&mut self) {
        for _ in 0..RESIZE_STEPS {
            // ERTS compares the unsigned counter with the shrink limit, so
            // a counter below 0 is a huge value and stops the shrink
            // (`erl_db_hash.c:3743-3760`).
            if self.size() >= u64::try_from(self.shrink_limit()).unwrap_or(u64::MAX) {
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
        if found.is_none() && self.is_fixed() {
            let key = self.key(&object);
            if let Some(records) = self.retained.get(&hash)
                && let Some(index) = records.iter().rposition(|retained| retained.key == *key)
            {
                match self.table_type {
                    TableType::Set | TableType::OrderedSet => self.revive(object, index, hash),
                    _ => {
                        // A bag or a duplicate_bag adds the object at the
                        // retained chain node, as ERTS adds it at the head
                        // of the object list (`erl_db_hash.c:1447-1452`).
                        let node = records[index].newest;
                        self.add(atoms, hash, object, node);
                    }
                }
                return;
            }
        }
        let node = match (found, self.table_type) {
            (Some(slot), TableType::Set | TableType::OrderedSet) => {
                let old = stored_hash(atoms, self.key(self.object_of(slot)));
                self.rekey(slot, old, hash);
                self.objects[slot] = Some(object);
                self.writes += 1;
                return;
            }
            (Some(_), _) => self.chain_node(hash, self.key(&object)).unwrap_or(EMPTY),
            (None, _) => EMPTY,
        };
        self.add(atoms, hash, object, node);
    }

    /// Put the object `object` at the chain node of the retained key
    /// `index`, as the insert of a fixed ERTS table reuses a
    /// pseudo-deleted object for the same key (`erl_db_hash.c:49-60`): the
    /// key keeps its place in the bucket chain, and the other objects of
    /// the retained key are freed.
    fn revive(&mut self, object: Vec<Term>, index: usize, hash: u64) {
        let (retained, empty) = {
            let records = self
                .retained
                .get_mut(&hash)
                .expect("the insert found the retained key");
            let retained = records.remove(index);
            (retained, records.is_empty())
        };
        if empty {
            self.retained.remove(&hash);
            let bucket = stored_bucket(hash, self.buckets);
            let bucket_empty = {
                let hashes = self
                    .retained_buckets
                    .get_mut(&bucket)
                    .expect("a retained record has a bucket");
                hashes.retain(|stored| *stored != hash);
                hashes.is_empty()
            };
            let _ = bucket_empty.then(|| self.retained_buckets.remove(&bucket));
        }
        for slot in &retained.slots {
            self.deleted[*slot] = false;
            self.older[*slot] = EMPTY;
        }
        self.deleted_objects -= retained.slots.len();
        self.objects[retained.newest] = Some(object);
        self.index.insert(hash, retained.newest);
        self.writes += 1;
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
                    self.insert_bag(atoms, object, &mut busy.walks, false)
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
        self.insert_all_with(atoms, objects, false)
    }

    /// Insert the objects of `ets:insert_new/2`: for a bag key that the
    /// fixation keeps without a live object, the key-clash walk adds a new
    /// object instead of restoring an equal pseudo-deleted one
    /// (`erl_db_hash.c:1412-1422`).
    pub(super) fn insert_all_new(
        &mut self,
        atoms: &AtomTable,
        objects: Vec<Vec<Term>>,
    ) -> Vec<usize> {
        self.insert_all_with(atoms, objects, true)
    }

    fn insert_all_with(
        &mut self,
        atoms: &AtomTable,
        objects: Vec<Vec<Term>>,
        new: bool,
    ) -> Vec<usize> {
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
            .map(|object| self.insert_bag(atoms, object, &mut walks, new))
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
        new: bool,
    ) -> usize {
        let hash = stored_hash(atoms, self.key(&object));
        if self.is_fixed() && self.position(hash, self.key(&object)).is_none() {
            let key = self.key(&object).clone();
            if let Some(records) = self.retained.get(&hash)
                && let Some(index) = records.iter().rposition(|retained| retained.key == key)
            {
                if new {
                    // The key-clash walk of `ets:insert_new/2` adds a new
                    // object at the head of the key's list
                    // (`erl_db_hash.c:1412-1422`, `1447-1452`).
                    let node = records[index].newest;
                    self.add(atoms, hash, object, node);
                    return 0;
                }
                // ERTS compares the new object with each object of the key
                // that the fixation keeps, and each comparison that is not
                // equal uses one unit (`erl_db_hash.c:1422-1444`).
                let slots = &records[index].slots;
                let compared = slots
                    .iter()
                    .take_while(|slot| self.object_of(**slot).as_slice() != object.as_slice())
                    .count();
                if let Some(slot) = slots.get(compared).copied() {
                    // The equal object stays as a pseudo-deleted object of
                    // the key, so the insert restores it
                    // (`erl_db_hash.c:1426-1437`).
                    let key = records[index].key.clone();
                    self.restore_object(slot, hash, &key);
                    return compared;
                }
                let node = records[index].newest;
                self.add(atoms, hash, object, node);
                return compared;
            }
        }
        let walks = walks.entry(hash).or_default();
        let index = match (walks.iter()).position(|walk| walk.key == *self.key(&object)) {
            Some(index) => index,
            None => {
                let key = self.key(&object).clone();
                // ERTS compares the new object with the first object of
                // the key's list first, and that object stays the chain
                // node when it is pseudo-deleted (`erl_db_hash.c:1423-1442`).
                let cursor = self.chain_node(hash, &key).unwrap_or(EMPTY);
                walks.push(BagWalk::new(key, cursor));
                walks.len() - 1
            }
        };
        let walk = &mut walks[index];
        let object_hash = object_hash(&object);
        match walk.find(self, object_hash, &object) {
            Some(count) => {
                if let Some(slot) = walk.seen.get(&object_hash).and_then(|pairs| {
                    pairs
                        .iter()
                        .find(|(_, slot)| self.object_of(*slot).as_slice() == object)
                        .map(|(_, slot)| *slot)
                }) && !new
                    && self.is_deleted(slot)
                {
                    self.restore_object(slot, hash, &walk.key);
                    // The restored object is the first object of the key,
                    // so the next object of the run compares from it
                    // (`erl_db_hash.c:1431-1435`).
                    walk.cursor = slot;
                    walk.walked = 0;
                    walk.added = 0;
                    walk.seen.clear();
                }
                count
            }
            None => {
                let count = walk.added + walk.walked;
                let node = self.chain_node(hash, &walk.key).unwrap_or(EMPTY);
                let slot = self.add(atoms, hash, object, node);
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
    fn add(&mut self, atoms: &AtomTable, hash: u64, object: Vec<Term>, node: usize) -> usize {
        let key = self.key(&object).clone();
        let index_slot = self.position(hash, &key).unwrap_or(EMPTY);
        let slot = self.allocate();
        self.objects[slot] = Some(object);
        self.older[slot] = node;
        self.writes += 1;
        let bucket = self.bucket(hash);
        if index_slot == EMPTY {
            self.index.insert(hash, slot);
        } else {
            self.index.replace(hash, index_slot, slot);
        }
        if node == EMPTY {
            let last = self.chain_keys(bucket).last().unwrap_or(EMPTY);
            self.link(bucket, last, slot);
        } else {
            let before = self.key_before(bucket, node);
            self.next[slot] = std::mem::replace(&mut self.next[node], EMPTY);
            self.link(bucket, before, slot);
        }
        // `grow` splits up to 5 buckets in one call, until the item counter
        // is within the growth limit (`erl_db_hash.c:3648-3734`).
        let mut splits = 0;
        while self.item_count() > i64::try_from(self.buckets).unwrap_or(i64::MAX)
            && splits < RESIZE_STEPS
        {
            self.split(atoms);
            splits += 1;
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
        stored_bucket(hash, self.buckets)
    }

    /// Drop the retained records of the buckets `first..last`, whose objects
    /// a segment free removed (`db_free_table_continue_hash`,
    /// `erl_db_hash.c:3140-3149`).
    fn forget_retained_segment(&mut self, first: usize, last: usize) {
        let buckets = (self.retained_buckets.range(first..last))
            .map(|(bucket, _)| *bucket)
            .collect::<Vec<_>>();
        for bucket in buckets {
            let hashes = self
                .retained_buckets
                .remove(&bucket)
                .expect("a collected bucket has records");
            for hash in hashes {
                self.retained.remove(&hash);
            }
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
        // ERTS does not resize a fixated table (`erl_db_hash.c:49-60`).
        if self.is_fixed() {
            return;
        }
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
        // ERTS does not resize a fixated table (`erl_db_hash.c:49-60`).
        if self.is_fixed() {
            return;
        }
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
            if !self.is_deleted(slot) {
                objects.push(self.object_of(slot).as_slice());
            }
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
        self.chain_keys(bucket).flat_map(|node| {
            links(node, &self.older)
                .filter(|&slot| !self.is_deleted(slot))
                .map(|slot| self.object_of(slot).as_slice())
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
            + self.fixed_by.capacity() * std::mem::size_of::<(ProcessId, usize)>()
            + self.retained.capacity() * std::mem::size_of::<(u64, Vec<Retained>)>()
            + self
                .retained
                .values()
                .map(|records| records.capacity() * std::mem::size_of::<Retained>())
                .sum::<usize>()
            + self.retained_buckets.len()
                * (std::mem::size_of::<usize>() + std::mem::size_of::<Vec<u64>>())
            + self
                .retained_buckets
                .values()
                .map(|hashes| hashes.capacity() * std::mem::size_of::<u64>())
                .sum::<usize>()
            + self
                .retained
                .values()
                .flatten()
                .map(|retained| retained.slots.capacity() * std::mem::size_of::<usize>())
                .sum::<usize>()
            + self.deleted.capacity() * std::mem::size_of::<bool>()
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
        self.objects.len() - self.free.len() - self.deleted_objects
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
            Some(slot) => {
                // A freed slot can hold the flag of a pseudo-deleted object
                // that a take freed (`db_take_hash`, `erl_db_hash.c:2949-2995`).
                if let Some(deleted) = self.deleted.get_mut(slot) {
                    *deleted = false;
                }
                slot
            }
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
        self.writes += 1;
        // An ordered_set has no supported traversal, and its compaction
        // moves the last object into the freed slot, so a fixed ordered_set
        // deletes as an unfixed one does (row "ETS tables" of
        // docs/erts-differences.md).
        if self.is_fixed() && self.table_type != TableType::OrderedSet {
            let node = self
                .chain_node(hash, key)
                .expect("a live key has a chain node");
            self.retain_key(hash, key, node);
            return;
        }
        self.index.remove(hash, newest);
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
        if self.table_type == TableType::OrderedSet {
            self.compact(atoms, newest);
        }
        for _ in 0..RESIZE_STEPS {
            // ERTS compares the unsigned counter with the shrink limit, so
            // a counter below 0 is a huge value and stops the shrink
            // (`erl_db_hash.c:3743-3760`).
            if self.size() >= u64::try_from(self.shrink_limit()).unwrap_or(u64::MAX) {
                break;
            }
            self.join();
        }
    }

    /// Move the object of the last slot into the slot `freed` of the
    /// `ordered_set` object that [`Table::delete`] just freed, and remove
    /// the last slot, so the slots of the table stay dense. The moved object
    /// keeps its key and its place in the bucket chain, and its slot in the
    /// key index changes. The free of a deleted `ordered_set` table then
    /// drops one slot for each object of the table and no slot of an object
    /// that an earlier delete freed, as the ERTS tree walk reads the live
    /// nodes only (`do_free_tree_continue`, `erl_db_tree.c:2877-2904`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::OrderedSet);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1)]);
    /// table.insert(&atoms, vec![Term::Integer(2)]);
    /// table.delete(&atoms, &Term::Integer(1));
    /// assert_eq!(table.objects.len(), 1);
    /// ```
    fn compact(&mut self, atoms: &AtomTable, freed: usize) {
        // The free list holds the slot that the caller freed. The pop runs in
        // each build, also a release build without `debug_assert!`.
        let freed_slot = self.free.pop();
        debug_assert_eq!(freed_slot, Some(freed));
        let last = self.objects.len() - 1;
        if freed == last {
            self.objects.pop();
            self.older.pop();
            self.next.pop();
            return;
        }
        let moved = self.object_of(last);
        let moved_hash = stored_hash(atoms, self.key(moved));
        self.objects[freed] = self.objects.pop().expect("the last slot is live");
        self.older[freed] = self.older.pop().expect("the last slot is allocated");
        self.index.remove(moved_hash, last);
        self.index.insert(moved_hash, freed);
        let bucket = self.bucket(moved_hash);
        // The chain walk reads `next[last]`, so the chain moves before the
        // array loses its last slot.
        match self.key_before(bucket, last) {
            EMPTY => self.heads[bucket] = freed,
            before => self.next[before] = freed,
        }
        self.next[freed] = self.next.pop().expect("the last slot is allocated");
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
        self.objects
            .iter()
            .enumerate()
            .filter(|(slot, _)| !self.is_deleted(*slot))
            .filter_map(|(_, object)| object.clone())
            .collect()
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
