//! ETS tables of the node, as the ERTS table store (`erl_db.c`). All
//! processes of a run share one store.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};

use crate::term::{Atom, AtomTable, LiveBytes, ProcessId, Reference, Term, internal_hash};

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
    /// The table that the exit of each owner frees, by owner, when the exit
    /// switched out in it: the reductions of each segment that is not free
    /// yet, the top segment last. ERTS keeps the table in the yield state of
    /// `erts_db_process_exiting` (`erl_db.c:5033-5041`, `5133-5143`). The
    /// table has no name and no reference at that time.
    freeing: HashMap<ProcessId, Vec<i32>>,
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
    pub(super) fn create(&mut self, id: Reference, name: Option<&str>, table: Table) {
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
        for table in self.tables.values() {
            live.add(table.allocated_bytes());
            for object in table.objects.iter().flatten() {
                for term in object {
                    live.term(term);
                }
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

    /// Delete the tables that the process `owner` owns, as
    /// `erts_db_process_exiting` (`erl_db.c:5031-5144`) does when the owner
    /// exits, with `reds` reductions left in the turn. Return the reductions
    /// that are left when each table is free, or `None` when the exit must
    /// switch out and call again in its next turn.
    ///
    /// The tables go in creation order (`save_owned_table`,
    /// `erl_db.c:628-650`). A table first loses its name and its reference,
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
    /// assert_eq!(tables.delete_owned(&AtomTable::new(), ProcessId(1), 100), Some(68));
    /// assert!(!tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn delete_owned(
        &mut self,
        atoms: &AtomTable,
        owner: ProcessId,
        mut reds: i32,
    ) -> Option<i32> {
        loop {
            if let Some(segments) = self.freeing.get_mut(&owner) {
                while let Some(segment) = segments.pop() {
                    reds -= segment;
                    if reds < 0 {
                        return None;
                    }
                }
                self.freeing.remove(&owner);
                if reds == 0 {
                    return None;
                }
            }
            let Some(id) = self
                .tables
                .iter()
                .filter(|(_, table)| table.owner == owner)
                .map(|(id, _)| *id)
                .min_by_key(|id| id.0)
            else {
                return Some(reds);
            };
            let table = self.tables.remove(&id).expect("an owned table is live");
            self.names.retain(|_, named| *named != id);
            self.freeing.insert(owner, table.segment_reductions(atoms));
        }
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
            buckets: FIRST_SEGMENT_SLOTS,
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
            (self.position(hash, key), hash)
        };
        let newest = match (found, self.table_type) {
            (Some(slot), TableType::Set) => {
                self.objects[slot] = Some(object);
                return;
            }
            (Some(newest), _) => newest,
            (None, _) => EMPTY,
        };
        self.add(hash, object, newest);
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
        let mut counts = Vec::with_capacity(objects.len());
        for object in objects {
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
            let count = match walk.find(self, object_hash, &object) {
                Some(count) => count,
                None => {
                    let count = walk.added + walk.walked;
                    let newest = self.position(hash, &walk.key).unwrap_or(EMPTY);
                    let slot = self.add(hash, object, newest);
                    walk.seen
                        .entry(object_hash)
                        .or_default()
                        .push((Place::Added(walk.added), slot));
                    walk.added += 1;
                    count
                }
            };
            counts.push(count);
        }
        counts
    }

    /// Put `object` into a new slot as the newest object of its key, whose
    /// stored key hash is `hash`, before the object in the slot `newest`, or
    /// as the only object of its key when `newest` is `EMPTY`. Return the
    /// slot. A new object that makes the objects more than the buckets
    /// splits one bucket (`erl_db_hash.c:1453-1461`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// let key = Term::Integer(1);
    /// assert_eq!(table.add(stored_hash(&atoms, &key), vec![key], EMPTY), 0);
    /// ```
    fn add(&mut self, hash: u64, object: Vec<Term>, newest: usize) -> usize {
        let slot = self.allocate();
        self.objects[slot] = Some(object);
        self.older[slot] = newest;
        if newest == EMPTY {
            self.index.insert(hash, slot);
        } else {
            self.index.replace(hash, newest, slot);
        }
        self.buckets = self.buckets.max(self.object_count());
        slot
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
        let slot = self.position(stored_hash(atoms, key), key)?;
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
        let mut slot = self.position(stored_hash(atoms, key), key).unwrap_or(EMPTY);
        while slot != EMPTY {
            objects.push(self.object_of(slot).as_slice());
            slot = self.older[slot];
        }
        objects.reverse();
        objects
    }

    /// Return the bytes of the container arrays of the table: the slot array
    /// of the objects, the free-slot array, and the element array of each
    /// stored object, and the array of the older slot of each slot. The number excludes the header of the `Vec` of each
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
            + (self.free.capacity() + self.older.capacity()) * std::mem::size_of::<usize>()
            + self
                .objects
                .iter()
                .flatten()
                .map(|object| object.capacity() * std::mem::size_of::<Term>())
                .sum::<usize>()
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
        let hash = stored_hash(atoms, key);
        let Some(newest) = self.position(hash, key) else {
            return;
        };
        self.index.remove(hash, newest);
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
            self.buckets -= 1;
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

    /// Return the reductions that the exit of the owner uses for each
    /// segment of the table, the top segment last: 32, and 1 for each object
    /// in the segment (`erl_db_hash.c:3141`, `3468-3530`). An object is in
    /// the slot of its key hash under the size mask, or under the half mask
    /// when that slot is not active (`hash_to_ix`, `erl_db_hash.c:199-209`).
    /// The size mask is one less than the power of two that holds the
    /// buckets (`erl_db_hash.c:3675-3681`, `3776-3779`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1, TableType::Set);
    /// let atoms = AtomTable::new();
    /// table.insert(&atoms, vec![Term::Integer(1)]);
    /// assert_eq!(table.segment_reductions(&atoms), vec![33]);
    /// ```
    fn segment_reductions(&self, atoms: &AtomTable) -> Vec<i32> {
        let mask = self.buckets.next_power_of_two() - 1;
        let mut segments = vec![SEGMENT_REDUCTIONS; segment(self.segment_slots() - 1) + 1];
        for object in self.objects.iter().flatten() {
            // `HashValue` is a `UWord`, so the key hash fits.
            let mut slot = stored_hash(atoms, self.key(object)) as usize & mask;
            if slot >= self.buckets {
                slot &= mask >> 1;
            }
            segments[segment(slot)] += 1;
        }
        segments
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

/// Return the atom table hash of `atom`, as `atom_hash` (`atom.c:83-105`)
/// gives it: the `hashpjw` hash of the UTF-8 bytes of the name, which this
/// function reads from `atoms`. A two-byte UTF-8 sequence for a Latin-1
/// character hashes as the low byte of that character.
///
/// # Examples
///
/// ```rust,ignore
/// let mut atoms = AtomTable::new();
/// let ab = atoms.put("ab").unwrap();
/// let e_acute = atoms.put("\u{e9}").unwrap();
/// assert_eq!(atom_hash(&atoms, &ab), 0x672);
/// assert_eq!(atom_hash(&atoms, &e_acute), 0xe9);
/// ```
fn atom_hash(atoms: &AtomTable, atom: &Atom) -> u64 {
    let mut bytes = atoms.name_of(atom).bytes().peekable();
    let mut hash: u64 = 0;
    while let Some(mut byte) = bytes.next() {
        if byte & 0xFE == 0xC2
            && let Some(next) = bytes.next_if(|next| next & 0xC0 == 0x80)
        {
            byte = (byte << 6) | (next & 0x3F);
        }
        hash = (hash << 4) + u64::from(byte);
        let high = hash & 0xF000_0000;
        if high != 0 {
            hash ^= high >> 24;
            hash ^= high;
        }
    }
    hash
}

#[cfg(test)]
#[path = "../../tests/runtime/ets.rs"]
mod tests;
