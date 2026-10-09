//! ETS tables of the node, as the ERTS table store (`erl_db.c`). All
//! processes of a run share one store.

use std::collections::HashMap;

use crate::term::{ProcessId, Reference, Term, internal_hash};

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

/// The ETS tables of the node. ERTS identifies each table by a table
/// reference that `make_btid` (`erl_db.c:2811`) gives it, also a named
/// table, and finds a named table by its atom in the meta name table. An
/// atom name is unique in one atom table, so its text identifies it, as in
/// `Scheduler::names`. PBeam supports only `set` tables.
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

/// One ETS `set` table.
#[derive(Debug)]
pub(super) struct Table {
    /// The process that owns the table, as `tb->common.owner`.
    owner: ProcessId,
    /// Which other processes can read and write the table.
    access: Access,
    /// The 1-based key position, as the C `int` `tb->common.keypos`
    /// (`erl_db_util.h:346`). `ets:new/2` accepts each small integer above
    /// 0, and the C assignment keeps only its low 32 bits, so the value can
    /// be 0 or negative.
    keypos: i32,
    /// The elements of each object. ERTS keeps a `set` in a hash table and
    /// finds a key with the `EQ` exact equality (`erl_db_hash.c:766-789`).
    /// `Term` has no hash, so this store uses a linear search with the exact
    /// equality of `Term`. Each object has a key: an element at `keypos`.
    objects: Vec<Vec<Term>>,
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
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Public, 1));
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
    /// tables.create(Reference(0), None, Table::new(ProcessId(0), Access::Protected, 1));
    /// tables.create(Reference(1), Some("ac_tab"), Table::new(ProcessId(0), Access::Public, 1));
    /// assert!(tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn create(&mut self, id: Reference, name: Option<&str>, table: Table) {
        self.tables.insert(id, table);
        if let Some(name) = name {
            self.names.insert(name.to_owned(), id);
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
    /// tables.create(Reference(0), Some("logger"), Table::new(ProcessId(0), Access::Private, 1));
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
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Protected, 1));
    /// let name = Term::static_atom("ac_tab");
    /// assert!(tables.table(&name, ProcessId(1), Operation::Read).is_some());
    /// assert!(tables.table(&name, ProcessId(1), Operation::Write).is_none());
    /// assert!(tables.table(&name, ProcessId(0), Operation::Write).is_some());
    /// assert!(tables.table(&Term::Integer(1), ProcessId(0), Operation::Read).is_none());
    /// ```
    pub(super) fn table(
        &mut self,
        id: &Term,
        caller: ProcessId,
        operation: Operation,
    ) -> Option<&mut Table> {
        let (id, _) = self.find(id)?;
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
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(0), Access::Private, 1));
    /// assert!(tables.find(&Term::static_atom("ac_tab")).is_some_and(|(id, _)| id == Reference(0)));
    /// assert!(tables.find(&Term::Reference(Reference(1))).is_none());
    /// assert!(tables.find(&Term::Integer(1)).is_none());
    /// ```
    pub(super) fn find(&self, id: &Term) -> Option<(Reference, &Table)> {
        let id = match id {
            Term::Atom(name) => *self.names.get(name.name())?,
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
    /// tables.create(Reference(0), Some("ac_tab"), Table::new(ProcessId(1), Access::Public, 1));
    /// assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(68));
    /// assert!(!tables.name_in_use("ac_tab"));
    /// ```
    pub(super) fn delete_owned(&mut self, owner: ProcessId, mut reds: i32) -> Option<i32> {
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
            self.freeing.insert(owner, table.segment_reductions());
        }
    }
}

impl Table {
    /// Return an empty table that the process `owner` owns, with the access
    /// mode `access` and the truncated key position `keypos`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let table = Table::new(ProcessId(0), Access::Protected, 1);
    /// assert_eq!(table.keypos(), 1);
    /// ```
    pub(super) fn new(owner: ProcessId, access: Access, keypos: i32) -> Table {
        Table {
            owner,
            access,
            keypos,
            objects: Vec::new(),
            buckets: FIRST_SEGMENT_SLOTS,
        }
    }

    /// Return the key position, as the C `int` of ERTS.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 2).keypos(), 2);
    /// ```
    pub(super) fn keypos(&self) -> i32 {
        self.keypos
    }

    /// Add the object with the elements `object`. An object with an exactly
    /// equal key replaces the old object, as `db_put_hash`
    /// (`erl_db_hash.c:1387-1402`) does in a `set` table. A new key that
    /// makes the objects more than the buckets splits one bucket
    /// (`erl_db_hash.c:1349-1354`). The buckets are never fewer than the
    /// objects, so one split is enough. The caller makes sure that the key
    /// position is at least 1 and at most the size of the object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// table.insert(vec![Term::static_atom("k"), Term::Integer(1)]);
    /// table.insert(vec![Term::static_atom("k"), Term::Integer(2)]);
    /// assert_eq!(table.object(&Term::static_atom("k")), Some(&[Term::static_atom("k"), Term::Integer(2)][..]));
    /// ```
    pub(super) fn insert(&mut self, object: Vec<Term>) {
        let key = self.key(&object);
        match self.objects.iter().position(|other| self.key(other) == key) {
            Some(index) => self.objects[index] = object,
            None => {
                self.objects.push(object);
                self.buckets = self.buckets.max(self.objects.len());
            }
        }
    }

    /// Return the elements of the object with a key that is exactly equal to
    /// `key`, or `None` when the table has no such object.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// assert_eq!(table.object(&Term::static_atom("k")), None);
    /// table.insert(vec![Term::static_atom("k")]);
    /// assert_eq!(table.object(&Term::static_atom("k")), Some(&[Term::static_atom("k")][..]));
    /// ```
    pub(super) fn object(&self, key: &Term) -> Option<&[Term]> {
        self.objects
            .iter()
            .find(|object| self.key(object) == key)
            .map(Vec::as_slice)
    }

    /// Remove the object with a key that is exactly equal to `key`, as
    /// `db_erase_hash` (`erl_db_hash.c:1613-1664`) does in a `set` table. A
    /// key that the table does not hold changes nothing. A removed object
    /// joins the top bucket into its low bucket, up to 5 times, while the
    /// objects are fewer than the shrink limit (`try_shrink` and `shrink`,
    /// `erl_db_hash.c:757-762`, `3743-3818`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// table.insert(vec![Term::static_atom("k")]);
    /// table.delete(&Term::static_atom("missing"));
    /// table.delete(&Term::static_atom("k"));
    /// assert_eq!(table.object(&Term::static_atom("k")), None);
    /// ```
    pub(super) fn delete(&mut self, key: &Term) {
        if let Some(index) = self
            .objects
            .iter()
            .position(|object| self.key(object) == key)
        {
            self.objects.remove(index);
            for _ in 0..RESIZE_STEPS {
                if self.objects.len() >= self.shrink_limit() {
                    break;
                }
                self.buckets -= 1;
            }
        }
    }

    /// Return the number of slots of the table: the first segment and each
    /// later segment that has an active bucket. A split allocates a segment
    /// when the buckets fill the slots (`erl_db_hash.c:3667-3672`), and a
    /// join frees the top segment when it has no active bucket
    /// (`erl_db_hash.c:3783-3785`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1).slots(), 256);
    /// ```
    fn slots(&self) -> usize {
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
    /// assert_eq!(Table::new(ProcessId(0), Access::Public, 1).shrink_limit(), 0);
    /// ```
    fn shrink_limit(&self) -> usize {
        let slots = self.slots();
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
    /// let mut table = Table::new(ProcessId(0), Access::Public, 1);
    /// table.insert(vec![Term::Integer(1)]);
    /// assert_eq!(table.segment_reductions(), vec![33]);
    /// ```
    fn segment_reductions(&self) -> Vec<i32> {
        let mask = self.buckets.next_power_of_two() - 1;
        let mut segments = vec![SEGMENT_REDUCTIONS; segment(self.slots() - 1) + 1];
        for object in &self.objects {
            // `HashValue` is a `UWord`, so the key hash fits.
            let mut slot = (key_hash(self.key(object)) >> 1) as usize & mask;
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
    /// let table = Table::new(ProcessId(0), Access::Public, 2);
    /// assert_eq!(table.key(&[Term::static_atom("v"), Term::static_atom("k")]), &Term::static_atom("k"));
    /// ```
    fn key<'a>(&self, object: &'a [Term]) -> &'a Term {
        let index = usize::try_from(self.keypos).expect("a stored object has a key position") - 1;
        &object[index]
    }
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
/// internal hash.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(key_hash(&Term::static_atom("a")), 0x61);
/// assert_eq!(key_hash(&Term::Integer(1)), 7938660269479274665);
/// ```
fn key_hash(key: &Term) -> u64 {
    match key {
        Term::Atom(atom) => atom_hash(atom.name()),
        _ => internal_hash(key),
    }
}

/// Return the atom table hash of the atom text `name`, as `atom_hash`
/// (`atom.c:83-105`) gives it: the `hashpjw` hash of the UTF-8 bytes. A
/// two-byte UTF-8 sequence for a Latin-1 character hashes as the low byte
/// of that character.
///
/// # Examples
///
/// ```rust,ignore
/// assert_eq!(atom_hash("ab"), 0x672);
/// assert_eq!(atom_hash("\u{e9}"), 0xe9);
/// ```
fn atom_hash(name: &str) -> u64 {
    let mut bytes = name.bytes().peekable();
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
mod tests {
    use super::{Access, EtsTables, Table};
    use crate::term::{ProcessId, Reference, Term};

    #[test]
    fn deletes_only_the_tables_of_the_owner() {
        let mut tables = EtsTables::default();
        let table = |owner| Table::new(ProcessId(owner), Access::Protected, 1);
        tables.create(Reference(0), None, table(1));
        tables.create(Reference(1), Some("ets_probe_owned"), table(1));
        tables.create(Reference(2), Some("ets_probe_named"), table(2));
        tables.create(Reference(3), None, table(2));

        assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(36));
        assert!(!tables.name_in_use("ets_probe_owned"));
        assert!(tables.name_in_use("ets_probe_named"));
        let mut live: Vec<_> = tables.tables.keys().copied().collect();
        live.sort_by_key(|table| table.0);
        assert_eq!(live, vec![Reference(2), Reference(3)]);
    }

    #[test]
    fn finds_a_named_table_of_each_access_mode() {
        let mut tables = EtsTables::default();
        tables.create(
            Reference(0),
            None,
            Table::new(ProcessId(1), Access::Public, 1),
        );
        tables.create(
            Reference(1),
            Some("ets_probe_private"),
            Table::new(ProcessId(1), Access::Private, 1),
        );
        assert_eq!(tables.whereis("ets_probe_private"), Some(Reference(1)));
        assert_eq!(tables.whereis("ets_probe_missing"), None);
        assert_eq!(tables.delete_owned(ProcessId(1), 100), Some(36));
        assert_eq!(tables.whereis("ets_probe_private"), None);
    }

    #[test]
    fn deletes_only_the_object_with_an_exactly_equal_key() {
        let mut table = Table::new(ProcessId(0), Access::Public, 2);
        let object = |value, key| vec![Term::Integer(value), key];
        let tuple = |value| Term::Tuple(vec![Term::Integer(value)]);
        table.insert(object(1, tuple(1)));
        table.insert(object(2, tuple(2)));
        table.delete(&tuple(3));
        table.delete(&tuple(1));
        assert_eq!(table.object(&tuple(1)), None);
        assert_eq!(table.object(&tuple(2)), Some(&object(2, tuple(2))[..]));
    }
}
